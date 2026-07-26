//! Provider authentication routes: credential listing, API-key login, and the OAuth code flow.
//!
//! The server owns the OAuth authorization-code flow at its own HTTP boundary: `authorize` issues a
//! CSRF `state` and an authorization URL whose redirect target is this server's own `/auth/callback`
//! route, and the callback validates the state, exchanges the code for a token, and persists the
//! credential to the shared 0600 auth store. API-key credentials can also be set directly. Stored
//! secrets are written only through the sanctioned `providers::store_credential` sink and are never
//! echoed back: every response reports authenticated state, never the credential itself.

use std::collections::{BTreeSet, HashMap};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::{
    Json,
    extract::{Path as AxumPath, Query, State},
    http::{HeaderMap, StatusCode, header},
};
use serde::{Deserialize, Serialize};

use super::{ApiError, ServerState};

/// How long a started OAuth authorization may wait for its callback before it is expired.
const PENDING_AUTH_TTL: Duration = Duration::from_secs(600);
/// Bounded timeout for the token-endpoint exchange so a slow/unreachable endpoint never hangs.
const TOKEN_EXCHANGE_TIMEOUT: Duration = Duration::from_secs(20);

/// Pending OAuth authorizations awaiting their loopback callback, keyed by CSRF `state`.
#[derive(Clone, Default)]
pub(super) struct PendingAuth {
    inner: Arc<Mutex<HashMap<String, PendingAuthEntry>>>,
}

impl PendingAuth {
    fn insert(&self, state: String, entry: PendingAuthEntry) {
        let mut guard = self.inner.lock().expect("pending auth lock");
        guard.retain(|_, existing| existing.created_at.elapsed() < PENDING_AUTH_TTL);
        guard.insert(state, entry);
    }

    fn take(&self, state: &str) -> Option<PendingAuthEntry> {
        let mut guard = self.inner.lock().expect("pending auth lock");
        guard.retain(|_, existing| existing.created_at.elapsed() < PENDING_AUTH_TTL);
        guard.remove(state)
    }
}

#[derive(Clone)]
struct PendingAuthEntry {
    provider: String,
    token_endpoint: String,
    client_id: String,
    client_secret: Option<String>,
    redirect_uri: String,
    created_at: Instant,
}

/// `GET /auth` — list provider auth methods and which providers are currently authenticated.
///
/// Methods come from the offline model catalogue plus every connectable profile that is not already
/// a catalogue row (so subscription OAuth providers like `chatgpt` still appear). Catalogue rows keep
/// the historical `api`+`oauth` type pair for SDK compatibility; profile-only rows advertise `oauth`
/// only when the profile declares `supports_oauth`.
pub(super) async fn list_auth(
    State(state): State<ServerState>,
) -> Result<Json<AuthListResponse>, ApiError> {
    let stored = stored_provider_set(&state);
    let mut methods: Vec<AuthMethod> = Vec::new();
    let mut seen = BTreeSet::new();

    for provider in providers::Catalogue::embedded().providers() {
        if !seen.insert(provider.id.clone()) {
            continue;
        }
        let env = provider.env.clone();
        let authenticated = stored.contains(&provider.id)
            || env.iter().any(|name| env_present(&state, name));
        methods.push(AuthMethod {
            provider: provider.id.clone(),
            name: provider.name.clone(),
            types: vec!["api".to_owned(), "oauth".to_owned()],
            env,
            authenticated,
        });
    }

    // Subscription / profile-only providers (chatgpt, …) are not always catalogue rows.
    for row in providers::connectable_providers() {
        if !seen.insert(row.id.to_owned()) {
            continue;
        }
        let mut types = vec!["api".to_owned()];
        if row.supports_oauth {
            types.push("oauth".to_owned());
        }
        let env = row
            .api_key_env
            .map(|name| vec![name.to_owned()])
            .unwrap_or_default();
        let authenticated =
            stored.contains(row.id) || env.iter().any(|name| env_present(&state, name));
        methods.push(AuthMethod {
            provider: row.id.to_owned(),
            name: row.display_name.to_owned(),
            types,
            env,
            authenticated,
        });
    }

    Ok(Json(AuthListResponse {
        authenticated: stored.into_iter().collect(),
        methods,
    }))
}

/// `PUT /auth/{provider}` — store an API-key or pre-obtained OAuth token for a provider.
pub(super) async fn set_auth(
    State(state): State<ServerState>,
    AxumPath(provider): AxumPath<String>,
    Json(payload): Json<SetAuthRequest>,
) -> Result<Json<AuthStateResponse>, ApiError> {
    let provider = normalized_provider(&provider)?;
    let kind = payload
        .kind
        .as_deref()
        .map(str::trim)
        .filter(|kind| !kind.is_empty())
        .unwrap_or("api");

    let secret = match kind {
        "api" => payload
            .key
            .as_deref()
            .or(payload.api_key.as_deref())
            .map(str::trim)
            .filter(|key| !key.is_empty())
            .ok_or_else(|| ApiError::bad_request("`key` is required for an api credential"))?
            .to_owned(),
        "oauth" => payload
            .access_token
            .as_deref()
            .or(payload.key.as_deref())
            .map(str::trim)
            .filter(|token| !token.is_empty())
            .ok_or_else(|| {
                ApiError::bad_request("`access_token` is required for an oauth credential")
            })?
            .to_owned(),
        other => {
            return Err(ApiError::bad_request(format!(
                "unsupported auth type `{other}` (expected `api` or `oauth`)"
            )));
        }
    };

    let path = require_auth_store_path(&state)?;
    providers::store_credential(&path, &provider, &secret).map_err(store_error)?;

    Ok(Json(AuthStateResponse {
        provider,
        authenticated: true,
        method: Some(kind.to_owned()),
    }))
}

/// `DELETE /auth/{provider}` — remove a stored credential (logout).
pub(super) async fn remove_auth(
    State(state): State<ServerState>,
    AxumPath(provider): AxumPath<String>,
) -> Result<Json<AuthStateResponse>, ApiError> {
    let provider = normalized_provider(&provider)?;
    let path = require_auth_store_path(&state)?;
    providers::remove_stored_credential(&path, &provider).map_err(store_error)?;
    Ok(Json(AuthStateResponse {
        provider,
        authenticated: false,
        method: None,
    }))
}

/// `POST /auth/{provider}/authorize` — begin the OAuth authorization-code flow.
///
/// Returns the authorization URL (carrying a fresh CSRF `state` and the loopback `redirect_uri`)
/// plus the `state` challenge. No secret is returned. The pending authorization is held until the
/// matching `/auth/callback` arrives.
pub(super) async fn oauth_authorize(
    State(state): State<ServerState>,
    AxumPath(provider): AxumPath<String>,
    headers: HeaderMap,
    Json(payload): Json<AuthorizeRequest>,
) -> Result<Json<AuthorizeResponse>, ApiError> {
    let provider = normalized_provider(&provider)?;
    let authorize_endpoint = required_field(&payload.authorize_endpoint, "authorize_endpoint")?;
    let token_endpoint = required_field(&payload.token_endpoint, "token_endpoint")?;
    let client_id = required_field(&payload.client_id, "client_id")?;
    let client_secret = payload
        .client_secret
        .as_deref()
        .map(str::trim)
        .filter(|secret| !secret.is_empty())
        .map(str::to_owned);

    let redirect_uri = match payload
        .redirect_uri
        .as_deref()
        .map(str::trim)
        .filter(|uri| !uri.is_empty())
    {
        Some(uri) => uri.to_owned(),
        None => callback_uri_from_host(&headers)?,
    };

    let challenge = random_token()?;
    let authorize_url = build_authorize_url(
        &authorize_endpoint,
        &client_id,
        &redirect_uri,
        &challenge,
        &payload.scopes,
    )?;

    state.pending_auth.insert(
        challenge.clone(),
        PendingAuthEntry {
            provider: provider.clone(),
            token_endpoint,
            client_id,
            client_secret,
            redirect_uri: redirect_uri.clone(),
            created_at: Instant::now(),
        },
    );

    Ok(Json(AuthorizeResponse {
        provider,
        authorize_url,
        state: challenge,
        redirect_uri,
    }))
}

/// `GET /auth/callback` — complete the OAuth flow: validate state, exchange the code, store a token.
pub(super) async fn oauth_callback(
    State(state): State<ServerState>,
    Query(query): Query<CallbackQuery>,
) -> Result<Json<AuthStateResponse>, ApiError> {
    if let Some(error) = query
        .error
        .as_deref()
        .map(str::trim)
        .filter(|error| !error.is_empty())
    {
        return Err(ApiError::bad_request(format!(
            "authorization server reported an error: {error}"
        )));
    }

    let challenge = query
        .state
        .as_deref()
        .map(str::trim)
        .filter(|state| !state.is_empty())
        .ok_or_else(|| ApiError::bad_request("callback is missing the state parameter"))?;
    let code = query
        .code
        .as_deref()
        .map(str::trim)
        .filter(|code| !code.is_empty())
        .ok_or_else(|| ApiError::bad_request("callback is missing the authorization code"))?;

    let entry = state
        .pending_auth
        .take(challenge)
        .ok_or_else(|| ApiError::bad_request("unknown or expired OAuth state (rejected)"))?;

    let token = exchange_code(&entry, code).await?;

    let path = require_auth_store_path(&state)?;
    providers::store_credential(&path, &entry.provider, &token.access_token)
        .map_err(store_error)?;

    Ok(Json(AuthStateResponse {
        provider: entry.provider,
        authenticated: true,
        method: Some("oauth".to_owned()),
    }))
}

/// Resolve the auth-store path from the server's captured configuration environment.
///
/// Mirrors `providers::AuthPathInputs` precedence (`CORTEX_AUTH_FILE` > `CORTEX_DATA_DIR` >
/// `${XDG_DATA_HOME:-$HOME/.local/share}/cortex/auth.json`) but reads from the server's config
/// snapshot so in-process tests can redirect the store without mutating the global environment.
pub(super) fn auth_store_path(config: &super::ConfigContext) -> Option<PathBuf> {
    let env_path = |name: &str| {
        config
            .env
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.trim())
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
    };
    let inputs = providers::AuthPathInputs {
        auth_file: env_path("CORTEX_AUTH_FILE"),
        data_dir: env_path("CORTEX_DATA_DIR"),
        xdg_data_home: env_path("XDG_DATA_HOME"),
        home: env_path("HOME").or_else(|| config.home.clone()),
    };
    inputs.resolve()
}

/// The set of providers that currently have a stored credential.
pub(super) fn stored_provider_set(state: &ServerState) -> BTreeSet<String> {
    let Some(path) = auth_store_path(&state.config) else {
        return BTreeSet::new();
    };
    providers::list_stored_providers(&path)
        .unwrap_or_default()
        .into_iter()
        .collect()
}

fn require_auth_store_path(state: &ServerState) -> Result<PathBuf, ApiError> {
    auth_store_path(&state.config).ok_or_else(|| {
        ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "cannot determine auth-store path; set HOME, XDG_DATA_HOME, CORTEX_DATA_DIR, or CORTEX_AUTH_FILE",
        )
    })
}

async fn exchange_code(entry: &PendingAuthEntry, code: &str) -> Result<TokenResponse, ApiError> {
    redaction::guard_outbound("OAuth token exchange")
        .map_err(|diagnostic| ApiError::diagnostic(StatusCode::SERVICE_UNAVAILABLE, diagnostic))?;

    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(TOKEN_EXCHANGE_TIMEOUT)
        .build()
        .map_err(|err| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("cannot build token-exchange client: {err}"),
            )
        })?;

    let mut form: Vec<(&str, &str)> = vec![
        ("grant_type", "authorization_code"),
        ("code", code),
        ("redirect_uri", entry.redirect_uri.as_str()),
        ("client_id", entry.client_id.as_str()),
    ];
    if let Some(secret) = entry.client_secret.as_deref() {
        form.push(("client_secret", secret));
    }

    let response = client
        .post(&entry.token_endpoint)
        .form(&form)
        .send()
        .await
        .map_err(|err| {
            ApiError::new(
                StatusCode::BAD_GATEWAY,
                format!("token endpoint request failed: {err}"),
            )
        })?;

    let status = response.status();
    let body = response.text().await.map_err(|err| {
        ApiError::new(
            StatusCode::BAD_GATEWAY,
            format!("cannot read token endpoint response: {err}"),
        )
    })?;
    if !status.is_success() {
        return Err(ApiError::new(
            StatusCode::BAD_GATEWAY,
            format!("token endpoint returned HTTP {}", status.as_u16()),
        ));
    }

    let token: TokenResponse = serde_json::from_str(&body).map_err(|err| {
        ApiError::new(
            StatusCode::BAD_GATEWAY,
            format!("invalid token endpoint response: {err}"),
        )
    })?;
    if token.access_token.trim().is_empty() {
        return Err(ApiError::new(
            StatusCode::BAD_GATEWAY,
            "token endpoint response did not include an access token",
        ));
    }
    Ok(token)
}

fn build_authorize_url(
    authorize_endpoint: &str,
    client_id: &str,
    redirect_uri: &str,
    state: &str,
    scopes: &[String],
) -> Result<String, ApiError> {
    let mut url = url::Url::parse(authorize_endpoint).map_err(|_| {
        ApiError::bad_request(format!(
            "invalid authorize_endpoint URL: {authorize_endpoint}"
        ))
    })?;
    {
        let mut query = url.query_pairs_mut();
        query.append_pair("response_type", "code");
        query.append_pair("client_id", client_id);
        query.append_pair("redirect_uri", redirect_uri);
        query.append_pair("state", state);
        let scope = scopes
            .iter()
            .map(|scope| scope.trim())
            .filter(|scope| !scope.is_empty())
            .collect::<Vec<_>>()
            .join(" ");
        if !scope.is_empty() {
            query.append_pair("scope", &scope);
        }
    }
    Ok(url.to_string())
}

fn callback_uri_from_host(headers: &HeaderMap) -> Result<String, ApiError> {
    let host = headers
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|host| !host.is_empty())
        .ok_or_else(|| {
            ApiError::bad_request(
                "authorize requires a loopback Host header or an explicit redirect_uri",
            )
        })?;
    Ok(format!("http://{host}/auth/callback"))
}

fn normalized_provider(raw: &str) -> Result<String, ApiError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(ApiError::bad_request("provider must be non-empty"));
    }
    Ok(trimmed.to_ascii_lowercase())
}

fn required_field(value: &Option<String>, name: &str) -> Result<String, ApiError> {
    value
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| ApiError::bad_request(format!("`{name}` is required")))
}

fn env_present(state: &ServerState, name: &str) -> bool {
    state
        .config
        .env
        .iter()
        .any(|(key, value)| key == name && !value.trim().is_empty())
}

fn store_error(err: std::io::Error) -> ApiError {
    ApiError::new(
        StatusCode::INTERNAL_SERVER_ERROR,
        format!("cannot update auth store: {err}"),
    )
}

/// A random hex token (128 bits) from the OS CSPRNG, used for the OAuth CSRF `state`.
///
/// This FAILS CLOSED: if the OS CSPRNG is unavailable there is no time-based fallback (a
/// `SystemTime`-nanos value would be predictable, making the single-use state guessable), so the
/// caller refuses the authorize request instead of issuing a weak challenge.
fn random_token() -> Result<String, ApiError> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).map_err(|err| {
        ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("cannot generate a secure OAuth state: {err}"),
        )
    })?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

#[derive(Debug, Deserialize)]
pub(super) struct SetAuthRequest {
    #[serde(default, rename = "type")]
    kind: Option<String>,
    #[serde(default)]
    key: Option<String>,
    #[serde(default)]
    api_key: Option<String>,
    #[serde(default)]
    access_token: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct AuthorizeRequest {
    #[serde(default)]
    authorize_endpoint: Option<String>,
    #[serde(default)]
    token_endpoint: Option<String>,
    #[serde(default)]
    client_id: Option<String>,
    #[serde(default)]
    client_secret: Option<String>,
    #[serde(default)]
    redirect_uri: Option<String>,
    #[serde(default)]
    scopes: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct CallbackQuery {
    #[serde(default)]
    code: Option<String>,
    #[serde(default)]
    state: Option<String>,
    #[serde(default)]
    error: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    #[serde(default)]
    #[allow(dead_code)]
    token_type: Option<String>,
    #[serde(default)]
    #[allow(dead_code)]
    refresh_token: Option<String>,
    #[serde(default)]
    #[allow(dead_code)]
    expires_in: Option<u64>,
}

#[derive(Debug, Serialize)]
pub(super) struct AuthListResponse {
    authenticated: Vec<String>,
    methods: Vec<AuthMethod>,
}

#[derive(Debug, Serialize)]
struct AuthMethod {
    provider: String,
    name: String,
    types: Vec<String>,
    env: Vec<String>,
    authenticated: bool,
}

#[derive(Debug, Serialize)]
pub(super) struct AuthStateResponse {
    provider: String,
    authenticated: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    method: Option<String>,
}

#[derive(Debug, Serialize)]
pub(super) struct AuthorizeResponse {
    provider: String,
    authorize_url: String,
    state: String,
    redirect_uri: String,
}
