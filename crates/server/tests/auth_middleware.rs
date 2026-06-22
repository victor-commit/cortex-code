use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode, header};
use tower::ServiceExt;

static SCRATCH_COUNTER: AtomicU64 = AtomicU64::new(0);

#[tokio::test]
async fn trusted_loopback_default_allows_protected_routes_without_auth() {
    let scratch = Scratch::new("server-auth-open-default");
    let app = server::app_with_db_path(scratch.db_path());

    let response = app
        .oneshot(
            Request::builder()
                .uri("/session")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("session list response");

    assert_eq!(response.status(), StatusCode::OK);
    assert_json_content_type(&response);
    let body: serde_json::Value = json_body(response).await;
    assert_eq!(body, serde_json::json!([]));
}

#[tokio::test]
async fn bearer_token_mode_rejects_missing_and_wrong_tokens_but_accepts_exact_token() {
    let scratch = Scratch::new("server-auth-token");
    let token = runtime_token("token-mode");
    let app = token_app(scratch.db_path(), &token, true);

    let missing = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/session")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("missing-token response");
    assert_eq!(missing.status(), StatusCode::UNAUTHORIZED);
    assert_json_content_type(&missing);
    let missing_body: serde_json::Value = json_body(missing).await;
    assert_eq!(missing_body["kind"], "auth");

    let wrong = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/session")
                .header(header::AUTHORIZATION, "Bearer wrong-token")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("wrong-token response");
    assert_eq!(wrong.status(), StatusCode::UNAUTHORIZED);

    let accepted = app
        .oneshot(
            Request::builder()
                .uri("/session")
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("accepted-token response");
    assert_eq!(accepted.status(), StatusCode::OK);
    let body: serde_json::Value = json_body(accepted).await;
    assert_eq!(body, serde_json::json!([]));
}

#[tokio::test]
async fn token_value_is_absent_from_responses_headers_and_openapi() {
    let scratch = Scratch::new("server-auth-no-leak");
    let token = runtime_token("no-leak");
    let app = token_app(scratch.db_path(), &token, true);

    let protected = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/session")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("protected response");
    let protected_headers = headers_to_string(protected.headers());
    let protected_body = body_string(protected).await;

    let health = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/health")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("health response");
    let health_headers = headers_to_string(health.headers());
    let health_body = body_string(health).await;

    let openapi = app
        .oneshot(
            Request::builder()
                .uri("/openapi.json")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("openapi response");
    let openapi_headers = headers_to_string(openapi.headers());
    let openapi_body = body_string(openapi).await;

    let captured = format!(
        "{protected_headers}\n{protected_body}\n{health_headers}\n{health_body}\n\
         {openapi_headers}\n{openapi_body}"
    );
    assert!(
        !captured.contains(&token),
        "configured server token leaked into response data:\n{captured}"
    );
    assert!(
        openapi_body.contains("Authorization: Bearer <TOKEN>"),
        "OpenAPI should document the bearer scheme without embedding the configured token"
    );
}

#[tokio::test]
async fn options_preflight_is_clean_and_cors_headers_follow_public_policy() {
    let scratch = Scratch::new("server-auth-preflight");
    let token = runtime_token("preflight");
    let app = token_app(scratch.db_path(), &token, true);

    let preflight = app
        .clone()
        .oneshot(
            Request::builder()
                .method("OPTIONS")
                .uri("/session")
                .header(header::ORIGIN, "http://localhost:9999")
                .header(header::ACCESS_CONTROL_REQUEST_METHOD, "POST")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("preflight response");
    assert_eq!(preflight.status(), StatusCode::NO_CONTENT);
    assert_eq!(
        preflight.headers().get(header::ACCESS_CONTROL_ALLOW_ORIGIN),
        Some(&"http://localhost:9999".parse().expect("origin header"))
    );
    let methods = preflight
        .headers()
        .get(header::ACCESS_CONTROL_ALLOW_METHODS)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("");
    assert!(methods.contains("POST"), "allow methods: {methods}");
    let headers = preflight
        .headers()
        .get(header::ACCESS_CONTROL_ALLOW_HEADERS)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("");
    assert!(
        headers.contains("authorization"),
        "allow headers: {headers}"
    );

    let health = app
        .oneshot(
            Request::builder()
                .uri("/health")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("health response");
    assert_eq!(health.status(), StatusCode::OK);
}

fn token_app(db_path: PathBuf, token: &str, public: bool) -> axum::Router {
    let auth = server::ServerAuth::bearer_token(token.to_owned(), public).expect("auth config");
    server::app_with_state(server::ServerState::new(db_path).with_auth(auth))
}

fn runtime_token(label: &str) -> String {
    let id = SCRATCH_COUNTER.fetch_add(1, Ordering::SeqCst);
    format!("server-token-{label}-{}-{id}", std::process::id())
}

fn assert_json_content_type(response: &axum::response::Response) {
    let content_type = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("");
    assert!(
        content_type.starts_with("application/json"),
        "expected application/json content type, got {content_type:?}"
    );
}

async fn json_body<T>(response: axum::response::Response) -> T
where
    T: serde::de::DeserializeOwned,
{
    let bytes = to_bytes(response.into_body(), 65_536).await.expect("body");
    serde_json::from_slice(&bytes).expect("JSON body")
}

async fn body_string(response: axum::response::Response) -> String {
    let bytes = to_bytes(response.into_body(), 65_536).await.expect("body");
    String::from_utf8(bytes.to_vec()).expect("UTF-8 body")
}

fn headers_to_string(headers: &axum::http::HeaderMap) -> String {
    headers
        .iter()
        .map(|(name, value)| format!("{name}: {value:?}"))
        .collect::<Vec<_>>()
        .join("\n")
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
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}
