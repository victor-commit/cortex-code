//! Black-box integration coverage for the config/provider/auth route family
//! (VAL-GAP-SERVER-007, VAL-GAP-SERVER-008): config get/patch, provider/config listing, auth
//! method listing, API-key login/logout, and the OAuth authorize + callback flow with a local
//! token-endpoint fixture. Secrets are never echoed and authenticated state persists across a
//! relaunch over the same store.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode, header};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tower::ServiceExt;

static SCRATCH_COUNTER: AtomicU64 = AtomicU64::new(0);

// ---- Config routes (VAL-GAP-SERVER-007) ----

#[tokio::test]
async fn get_config_reports_effective_values_and_redacts_secrets() {
    let scratch = Scratch::new("server-config-get");
    let workspace = scratch.workspace();
    let secret = fake_key();
    std::fs::write(
        workspace.join("cortex.toml"),
        format!(
            "model = \"anthropic/claude-sonnet-4\"\n\n[provider.openrouter]\napi_key = \"{secret}\"\n"
        ),
    )
    .expect("write project config");

    let app = server::app_with_state(
        server::ServerState::new(scratch.db_path())
            .with_cwd(workspace)
            .with_config_env(Vec::new()),
    );

    let response = send(&app, "GET", "/config", None).await;
    assert_eq!(response.status(), StatusCode::OK);
    let body_text = body_text(response).await;
    assert!(
        body_text.contains("anthropic/claude-sonnet-4"),
        "effective model should be reported: {body_text}"
    );
    assert!(
        !body_text.contains(&secret),
        "config dump must not leak the secret: {body_text}"
    );
    assert!(
        body_text.contains(redaction::PLACEHOLDER),
        "secret config value should be redacted: {body_text}"
    );
}

#[tokio::test]
async fn patch_config_updates_persisted_model_and_rejects_bad_input() {
    let scratch = Scratch::new("server-config-patch");
    let workspace = scratch.workspace();
    std::fs::write(
        workspace.join("cortex.toml"),
        "model = \"anthropic/claude-sonnet-4\"\n",
    )
    .expect("write project config");

    let make_app = || {
        server::app_with_state(
            server::ServerState::new(scratch.db_path())
                .with_cwd(workspace.clone())
                .with_config_env(Vec::new()),
        )
    };
    let app = make_app();

    // Patch a valid model; the response reflects the new value.
    let response = send(
        &app,
        "PATCH",
        "/config",
        Some(r#"{"model":"google/gemini-2.5-flash"}"#),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value = json_body(response).await;
    assert_eq!(body["model"], "google/gemini-2.5-flash");

    // Later GET over a fresh app on the same project reflects the persisted change.
    let relaunched = make_app();
    let response = send(&relaunched, "GET", "/config", None).await;
    let body: serde_json::Value = json_body(response).await;
    assert_eq!(body["model"], "google/gemini-2.5-flash");

    // Empty patch is a clean client error.
    let response = send(&app, "PATCH", "/config", Some("{}")).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    // Unknown field is rejected.
    let response = send(&app, "PATCH", "/config", Some(r#"{"db_path":"/tmp/x"}"#)).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    // Malformed model id is rejected before any write.
    let response = send(
        &app,
        "PATCH",
        "/config",
        Some(r#"{"model":"not a valid model id"}"#),
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    // Out-of-range max_steps is rejected.
    let response = send(&app, "PATCH", "/config", Some(r#"{"max_steps":0}"#)).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn config_providers_lists_catalog_with_default_model() {
    let scratch = Scratch::new("server-config-providers");
    let workspace = scratch.workspace();
    let app = server::app_with_state(
        server::ServerState::new(scratch.db_path())
            .with_cwd(workspace)
            .with_config_env(Vec::new()),
    );

    let response = send(&app, "GET", "/config/providers", None).await;
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value = json_body_large(response).await;
    assert!(
        body["default_model"].as_str().is_some_and(is_qualified),
        "default_model should be a fully-qualified id: {body}"
    );
    let providers = body["providers"].as_array().expect("providers array");
    let default_model = body["default_model"]
        .as_str()
        .expect("default model is a string");
    assert!(
        providers
            .iter()
            .flat_map(|provider| provider["models"].as_array().into_iter().flatten())
            .any(|model| model["id"] == default_model),
        "the provider catalogue should include its default model: {body}"
    );
    assert!(
        body["provider_config"].is_object(),
        "provider_config report should be present: {body}"
    );
}

// ---- Provider/auth routes (VAL-GAP-SERVER-008) ----

#[tokio::test]
async fn api_key_login_then_provider_shows_authenticated_without_leaking_secret() {
    let scratch = Scratch::new("server-auth-api");
    let workspace = scratch.workspace();
    let app = server::app_with_state(
        server::ServerState::new(scratch.db_path())
            .with_cwd(workspace)
            .with_config_env(vec![(
                "CORTEX_AUTH_FILE".to_owned(),
                scratch.auth_path().to_string_lossy().into_owned(),
            )]),
    );

    // Initially unauthenticated.
    let response = send(&app, "GET", "/provider", None).await;
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value = json_body_large(response).await;
    assert_eq!(
        provider_authenticated(&body, "openrouter"),
        Some(false),
        "openrouter starts unauthenticated: {body}"
    );

    // Store an API key; the response must not echo the secret.
    let secret = fake_key();
    let response = send(
        &app,
        "PUT",
        "/auth/openrouter",
        Some(&format!(r#"{{"type":"api","key":"{secret}"}}"#)),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let text = body_text(response).await;
    assert!(
        !text.contains(&secret),
        "auth response leaked secret: {text}"
    );
    let body: serde_json::Value = serde_json::from_str(&text).expect("json");
    assert_eq!(body["authenticated"], true);

    // Provider route now reports authenticated state.
    let response = send(&app, "GET", "/provider", None).await;
    let body: serde_json::Value = json_body_large(response).await;
    assert_eq!(provider_authenticated(&body, "openrouter"), Some(true));

    // Auth listing also reflects it and never carries the secret.
    let response = send(&app, "GET", "/auth", None).await;
    let text = body_text(response).await;
    assert!(!text.contains(&secret), "auth list leaked secret: {text}");
    let body: serde_json::Value = serde_json::from_str(&text).expect("json");
    assert!(
        body["authenticated"]
            .as_array()
            .is_some_and(|ids| ids.iter().any(|id| id == "openrouter")),
        "auth list should include the authenticated provider: {body}"
    );

    // The stored secret persists across a relaunch over the same store.
    let relaunched = server::app_with_state(
        server::ServerState::new(scratch.db_path())
            .with_cwd(scratch.workspace())
            .with_config_env(vec![(
                "CORTEX_AUTH_FILE".to_owned(),
                scratch.auth_path().to_string_lossy().into_owned(),
            )]),
    );
    let response = send(&relaunched, "GET", "/provider", None).await;
    let body: serde_json::Value = json_body_large(response).await;
    assert_eq!(provider_authenticated(&body, "openrouter"), Some(true));

    // Logout removes the credential.
    let response = send(&app, "DELETE", "/auth/openrouter", None).await;
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value = json_body(response).await;
    assert_eq!(body["authenticated"], false);
    let response = send(&app, "GET", "/provider", None).await;
    let body: serde_json::Value = json_body_large(response).await;
    assert_eq!(provider_authenticated(&body, "openrouter"), Some(false));
}

#[tokio::test]
async fn api_key_login_requires_a_non_empty_key() {
    let scratch = Scratch::new("server-auth-api-empty");
    let app = server::app_with_state(
        server::ServerState::new(scratch.db_path())
            .with_cwd(scratch.workspace())
            .with_config_env(vec![(
                "CORTEX_AUTH_FILE".to_owned(),
                scratch.auth_path().to_string_lossy().into_owned(),
            )]),
    );
    let response = send(&app, "PUT", "/auth/openrouter", Some(r#"{"type":"api"}"#)).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn oauth_authorize_then_callback_stores_token_and_authenticates() {
    let scratch = Scratch::new("server-auth-oauth");
    let (token_url, fixture) = start_token_fixture().await;
    let app = server::app_with_state(
        server::ServerState::new(scratch.db_path())
            .with_cwd(scratch.workspace())
            .with_config_env(vec![(
                "CORTEX_AUTH_FILE".to_owned(),
                scratch.auth_path().to_string_lossy().into_owned(),
            )]),
    );

    // Begin the OAuth flow: an authorize URL and CSRF state are returned (no secret).
    let authorize_body = serde_json::json!({
        "authorize_endpoint": "https://auth.example.test/authorize",
        "token_endpoint": token_url,
        "client_id": "cortex-fixture-client",
        "redirect_uri": "http://127.0.0.1:18765/auth/callback",
        "scopes": ["profile", "models"],
    })
    .to_string();
    let response = send(&app, "POST", "/auth/acme/authorize", Some(&authorize_body)).await;
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value = json_body(response).await;
    let state = body["state"].as_str().expect("state").to_owned();
    let authorize_url = body["authorize_url"].as_str().expect("authorize_url");
    assert!(authorize_url.contains("response_type=code"));
    assert!(authorize_url.contains("client_id=cortex-fixture-client"));
    assert!(
        authorize_url.contains(&format!("state={state}")),
        "authorize URL should carry the CSRF state: {authorize_url}"
    );

    // A callback with an unknown state is rejected (CSRF guard).
    let response = send(
        &app,
        "GET",
        "/auth/callback?code=abc&state=not-the-issued-state",
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    // The genuine callback exchanges the code at the fixture and stores the token.
    let response = send(
        &app,
        "GET",
        &format!("/auth/callback?code=fixture-code&state={state}"),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value = json_body(response).await;
    assert_eq!(body["provider"], "acme");
    assert_eq!(body["authenticated"], true);

    // The provider route now shows the authenticated state, without leaking the token.
    let response = send(&app, "GET", "/provider", None).await;
    let text = body_text(response).await;
    assert!(
        !text.contains("fixture-access-token"),
        "provider list leaked the OAuth token: {text}"
    );
    let body: serde_json::Value = serde_json::from_str(&text).expect("json");
    assert_eq!(provider_authenticated(&body, "acme"), Some(true));

    // Re-using the consumed state fails (single-use).
    let response = send(
        &app,
        "GET",
        &format!("/auth/callback?code=fixture-code&state={state}"),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    fixture.abort();
}

#[tokio::test]
async fn oauth_authorize_derives_callback_from_host_header() {
    let scratch = Scratch::new("server-auth-oauth-host");
    let app = server::app_with_state(
        server::ServerState::new(scratch.db_path())
            .with_cwd(scratch.workspace())
            .with_config_env(vec![(
                "CORTEX_AUTH_FILE".to_owned(),
                scratch.auth_path().to_string_lossy().into_owned(),
            )]),
    );
    let authorize_body = serde_json::json!({
        "authorize_endpoint": "https://auth.example.test/authorize",
        "token_endpoint": "https://auth.example.test/token",
        "client_id": "cortex-fixture-client",
    })
    .to_string();
    let request = Request::builder()
        .method("POST")
        .uri("/auth/acme/authorize")
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::HOST, "127.0.0.1:18776")
        .body(Body::from(authorize_body))
        .expect("request");
    let response = app.clone().oneshot(request).await.expect("response");
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value = json_body(response).await;
    assert_eq!(body["redirect_uri"], "http://127.0.0.1:18776/auth/callback");
}

#[tokio::test]
async fn oauth_authorize_requires_endpoints() {
    let scratch = Scratch::new("server-auth-oauth-missing");
    let app = server::app_with_state(
        server::ServerState::new(scratch.db_path())
            .with_cwd(scratch.workspace())
            .with_config_env(vec![(
                "CORTEX_AUTH_FILE".to_owned(),
                scratch.auth_path().to_string_lossy().into_owned(),
            )]),
    );
    let response = send(
        &app,
        "POST",
        "/auth/acme/authorize",
        Some(r#"{"client_id":"x"}"#),
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

// The OAuth CSRF `state` is drawn from the OS CSPRNG, not a predictable time-based source: each
// issued state is 128 bits of lowercase hex, every byte is random (the old time-nanos fallback left
// the high 8 bytes zero), and states never repeat. A predictable single-use state would let an
// attacker forge the callback, so this fails closed rather than degrade to a guessable value.
#[tokio::test]
async fn oauth_authorize_state_uses_the_csprng_not_a_time_fallback() {
    let scratch = Scratch::new("server-auth-oauth-entropy");
    let app = server::app_with_state(
        server::ServerState::new(scratch.db_path())
            .with_cwd(scratch.workspace())
            .with_config_env(vec![(
                "CORTEX_AUTH_FILE".to_owned(),
                scratch.auth_path().to_string_lossy().into_owned(),
            )]),
    );
    let authorize_body = serde_json::json!({
        "authorize_endpoint": "https://auth.example.test/authorize",
        "token_endpoint": "https://auth.example.test/token",
        "client_id": "cortex-fixture-client",
        "redirect_uri": "http://127.0.0.1:18765/auth/callback",
        "scopes": ["profile"],
    })
    .to_string();

    let mut states = std::collections::HashSet::new();
    let mut high_halves = std::collections::HashSet::new();
    let mut any_high_byte_nonzero = false;
    for _ in 0..32 {
        let response = send(&app, "POST", "/auth/acme/authorize", Some(&authorize_body)).await;
        assert_eq!(response.status(), StatusCode::OK);
        let body: serde_json::Value = json_body(response).await;
        let state = body["state"].as_str().expect("state").to_owned();
        assert_eq!(state.len(), 32, "128-bit state is 32 hex chars: {state}");
        assert!(
            state
                .chars()
                .all(|ch| ch.is_ascii_hexdigit() && !ch.is_ascii_uppercase()),
            "state is lowercase hex: {state}"
        );
        // Bytes 8..16 (hex 16..32) were always zero in the time-nanos fallback; the CSPRNG fills
        // them with random data.
        if state[16..].chars().any(|ch| ch != '0') {
            any_high_byte_nonzero = true;
        }
        high_halves.insert(state[16..].to_owned());
        assert!(states.insert(state), "each issued state is unique");
    }
    assert!(
        any_high_byte_nonzero,
        "the high bytes of the state must be random, not a zero-padded timestamp"
    );
    assert!(
        high_halves.len() > 1,
        "the high half of the state must vary across issuances (CSPRNG, not a fixed/zero suffix)"
    );
}

// ---- helpers ----

fn is_qualified(model: &str) -> bool {
    model
        .split_once('/')
        .is_some_and(|(provider, name)| !provider.trim().is_empty() && !name.trim().is_empty())
}

fn provider_authenticated(body: &serde_json::Value, provider_id: &str) -> Option<bool> {
    body["providers"]
        .as_array()?
        .iter()
        .find(|provider| provider["id"] == provider_id)
        .and_then(|provider| provider["authenticated"].as_bool())
}

/// A fake-but-key-shaped secret assembled from fragments so no contiguous key-shaped literal lives
/// in source (the commit secret scanner flags those).
fn fake_key() -> String {
    format!("sk-{}-{}", "or-v1", "abcd1234".repeat(6))
}

async fn start_token_fixture() -> (String, tokio::task::JoinHandle<()>) {
    let listener = bind_in_reserved_range().await;
    let port = listener.local_addr().expect("local addr").port();
    let url = format!("http://127.0.0.1:{port}/token");
    let handle = tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                break;
            };
            tokio::spawn(async move {
                let mut buffer = [0u8; 4096];
                let _ = stream.read(&mut buffer).await;
                let body = r#"{"access_token":"fixture-access-token","token_type":"Bearer","expires_in":3600}"#;
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = stream.write_all(response.as_bytes()).await;
                let _ = stream.flush().await;
            });
        }
    });
    (url, handle)
}

async fn bind_in_reserved_range() -> TcpListener {
    TcpListener::bind(("127.0.0.1", 0))
        .await
        .expect("bind ephemeral loopback listener")
}

async fn send(
    app: &axum::Router,
    method: &str,
    uri: &str,
    body: Option<&str>,
) -> axum::response::Response {
    let mut builder = Request::builder().method(method).uri(uri);
    let request = match body {
        Some(body) => builder
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(body.to_owned()))
            .expect("request"),
        None => {
            builder = builder.header(header::CONTENT_TYPE, "application/json");
            builder.body(Body::empty()).expect("request")
        }
    };
    app.clone().oneshot(request).await.expect("response")
}

async fn body_text(response: axum::response::Response) -> String {
    let bytes = to_bytes(response.into_body(), 262_144).await.expect("body");
    String::from_utf8(bytes.to_vec()).expect("utf8 body")
}

async fn json_body<T>(response: axum::response::Response) -> T
where
    T: serde::de::DeserializeOwned,
{
    let bytes = to_bytes(response.into_body(), 32_768).await.expect("body");
    serde_json::from_slice(&bytes).expect("JSON body")
}

async fn json_body_large<T>(response: axum::response::Response) -> T
where
    T: serde::de::DeserializeOwned,
{
    let bytes = to_bytes(response.into_body(), 1_048_576)
        .await
        .expect("body");
    serde_json::from_slice(&bytes).expect("JSON body")
}

struct Scratch {
    dir: PathBuf,
}

impl Scratch {
    fn new(label: &str) -> Self {
        let id = SCRATCH_COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("cortex-{label}-{}-{id}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("create scratch dir");
        Self { dir }
    }

    fn db_path(&self) -> PathBuf {
        self.dir.join("cortex.db")
    }

    fn workspace(&self) -> PathBuf {
        let workspace = self.dir.join("workspace");
        std::fs::create_dir_all(&workspace).expect("create workspace dir");
        workspace
    }

    fn auth_path(&self) -> PathBuf {
        self.dir.join("auth.json")
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}
