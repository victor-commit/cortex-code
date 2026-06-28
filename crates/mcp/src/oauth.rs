//! The MCP-remote OAuth 2.0 authorization-code flow with a CSRF-validated loopback callback.
//!
//! When a remote server answers the handshake with `401`/needs-auth, the user runs the auth flow:
//! cortex discovers the server's authorization and token endpoints, generates a random CSRF
//! `state`, starts a loopback HTTP listener on a port in the mission's reserved range (3200-3299),
//! and prints an authorization URL. The user (or an automated validator) visits that URL; the
//! server redirects back to the loopback callback with an authorization `code` and the `state`.
//! cortex validates the returned `state` against the one it generated (a forged/mismatched state is
//! rejected), exchanges the code for tokens at the token endpoint, and returns the credential to be
//! persisted. Expired credentials are refreshed in place via the refresh token.
//!
//! Token values are never logged or printed here — only the authorization URL (which carries no
//! secret) is surfaced.

use std::time::Duration;

use serde::Deserialize;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use crate::credentials::{StoredToken, now_secs};

/// The reserved loopback port range the OAuth callback binds within (mission boundary).
const CALLBACK_PORT_RANGE: std::ops::RangeInclusive<u16> = 3200..=3299;

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
    #[error("invalid OAuth callback port {0}: expected a port in 3200-3299")]
    InvalidCallbackPort(u16),
    /// The token endpoint response could not be parsed.
    #[error("invalid token response: {0}")]
    TokenResponse(String),
    /// The OS CSPRNG was unavailable, so a secure CSRF `state` could not be generated.
    #[error("cannot generate a secure OAuth state: {0}")]
    RandomSource(String),
}

/// The resolved OAuth endpoints and client identity for a remote server.
#[derive(Debug, Clone)]
pub struct OAuthConfig {
    /// The authorization endpoint (`/authorize`).
    pub authorize_endpoint: String,
    /// The token endpoint (`/token`).
    pub token_endpoint: String,
    /// The client id presented to the authorization server.
    pub client_id: String,
    /// The requested scopes (may be empty).
    pub scopes: Vec<String>,
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
    })
}

/// The result of starting the authorization flow: the URL to visit and the live callback listener.
pub struct AuthorizationRequest {
    /// The full authorization URL to open in a browser (carries the CSRF `state`, no secret).
    pub authorize_url: String,
    /// The loopback redirect URI the server will call back.
    pub redirect_uri: String,
    state: String,
    listener: TcpListener,
    config: OAuthConfig,
}

/// Begin the authorization flow: bind a loopback callback, generate the CSRF state, and build the
/// authorization URL. `callback_port` pins the loopback port (must be in 3200-3299); `None` scans
/// the reserved range for a free port.
pub async fn begin(
    config: OAuthConfig,
    callback_port: Option<u16>,
) -> Result<AuthorizationRequest, OAuthError> {
    let listener = bind_callback(callback_port).await?;
    let port = listener
        .local_addr()
        .map_err(|err| OAuthError::Callback(err.to_string()))?
        .port();
    let redirect_uri = format!("http://127.0.0.1:{port}/callback");
    let state = random_token()?;

    let mut authorize = url::Url::parse(&config.authorize_endpoint)
        .map_err(|_| OAuthError::InvalidUrl(config.authorize_endpoint.clone()))?;
    {
        let mut query = authorize.query_pairs_mut();
        query.append_pair("response_type", "code");
        query.append_pair("client_id", &config.client_id);
        query.append_pair("redirect_uri", &redirect_uri);
        query.append_pair("state", &state);
        if !config.scopes.is_empty() {
            query.append_pair("scope", &config.scopes.join(" "));
        }
    }

    Ok(AuthorizationRequest {
        authorize_url: authorize.to_string(),
        redirect_uri,
        state,
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
        exchange_code(&self.config, &code, &self.redirect_uri).await
    }
}

/// Bind a loopback TCP listener for the callback, either on `port` or the first free port in the
/// reserved range.
async fn bind_callback(port: Option<u16>) -> Result<TcpListener, OAuthError> {
    if let Some(port) = port {
        if !CALLBACK_PORT_RANGE.contains(&port) {
            return Err(OAuthError::InvalidCallbackPort(port));
        }
        return TcpListener::bind(("127.0.0.1", port))
            .await
            .map_err(|err| OAuthError::Callback(err.to_string()));
    }
    for candidate in CALLBACK_PORT_RANGE {
        if let Ok(listener) = TcpListener::bind(("127.0.0.1", candidate)).await {
            return Ok(listener);
        }
    }
    Err(OAuthError::NoCallbackPort)
}

/// Accept loopback connections until a `/callback` request arrives, validate its state, and return
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
        if path != "/callback" {
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

/// Exchange an authorization code for tokens at the token endpoint.
async fn exchange_code(
    config: &OAuthConfig,
    code: &str,
    redirect_uri: &str,
) -> Result<StoredToken, OAuthError> {
    let client = reqwest::Client::builder().build()?;
    let form = [
        ("grant_type", "authorization_code"),
        ("code", code),
        ("redirect_uri", redirect_uri),
        ("client_id", config.client_id.as_str()),
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

/// Parse a token endpoint response into a [`StoredToken`].
async fn parse_token_response(
    response: reqwest::Response,
    config: &OAuthConfig,
) -> Result<StoredToken, OAuthError> {
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

    let status = response.status();
    let text = response.text().await?;
    if !status.is_success() {
        return Err(OAuthError::Authorization(format!(
            "token endpoint returned HTTP {status}"
        )));
    }
    let body: TokenBody =
        serde_json::from_str(&text).map_err(|err| OAuthError::TokenResponse(err.to_string()))?;
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
}
