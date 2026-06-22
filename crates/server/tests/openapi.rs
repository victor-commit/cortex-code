use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode, header};
use tower::ServiceExt;

static SCRATCH_COUNTER: AtomicU64 = AtomicU64::new(0);

#[tokio::test]
async fn openapi_route_returns_valid_document_with_implemented_paths() {
    let scratch = Scratch::new("server-openapi");
    let app = server::app_with_db_path(scratch.db_path());

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/openapi.json")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("openapi response");

    assert_eq!(response.status(), StatusCode::OK);
    assert_json_content_type(&response);
    let document: serde_json::Value = json_body(response).await;

    assert!(
        document
            .get("openapi")
            .or_else(|| document.get("swagger"))
            .is_some(),
        "document should declare an OpenAPI/Swagger version: {document:?}"
    );
    assert!(
        document["info"].is_object(),
        "info section required: {document:?}"
    );
    assert!(
        document["paths"].is_object(),
        "paths object required: {document:?}"
    );

    let paths = document["paths"].as_object().expect("paths object");
    assert_method(paths, "/health", "get", 200);
    assert_method(paths, "/event", "get", 200);
    assert_method(paths, "/event", "post", 200);
    assert_method(paths, "/session", "get", 200);
    assert_method(paths, "/session", "post", 201);
    assert_method(paths, "/session/{id}", "get", 200);
    assert_method(paths, "/session/{id}", "delete", 204);
    assert_method(paths, "/session/{id}/message", "get", 200);
    assert_method(paths, "/session/{id}/message", "post", 200);
    assert_method(paths, "/session/{id}/prompt", "get", 200);
    assert_method(paths, "/session/{id}/prompt/append", "post", 200);
    assert_method(paths, "/session/{id}/prompt/submit", "post", 200);
    assert_method(paths, "/session/{id}/prompt/clear", "post", 200);
    assert_method(paths, "/session/{id}/command", "post", 200);
    assert_method(paths, "/session/{id}/abort", "post", 200);
    assert_method(paths, "/session/{id}/share", "post", 200);
    assert_method(paths, "/session/{id}/unshare", "post", 200);
    assert_method(paths, "/share/{share_id}", "get", 200);
    assert_eq!(
        document["components"]["securitySchemes"]["bearerAuth"]["scheme"], "bearer",
        "OpenAPI should document the Authorization bearer scheme"
    );
    assert_protected_operation(&document, "/session", "get");
    assert_protected_operation(&document, "/session/{id}/message", "post");
    assert_protected_operation(&document, "/event", "get");
    assert!(
        !document.to_string().contains("server-token"),
        "OpenAPI must not embed configured token values"
    );

    for path in paths.keys() {
        assert!(
            matches!(
                path.as_str(),
                "/health"
                    | "/event"
                    | "/session"
                    | "/session/{id}"
                    | "/session/{id}/message"
                    | "/session/{id}/prompt"
                    | "/session/{id}/prompt/append"
                    | "/session/{id}/prompt/submit"
                    | "/session/{id}/prompt/clear"
                    | "/session/{id}/command"
                    | "/session/{id}/abort"
                    | "/session/{id}/share"
                    | "/session/{id}/unshare"
                    | "/share/{share_id}"
                    | "/openapi.json"
            ),
            "OpenAPI document should not advertise phantom path {path:?}"
        );
    }
}

#[tokio::test]
async fn openapi_documented_gets_match_real_routes_and_unknown_paths_404() {
    let scratch = Scratch::new("server-openapi-behavior");
    let app = server::app_with_db_path(scratch.db_path());

    let document_response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/openapi.json")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("openapi response");
    assert_eq!(document_response.status(), StatusCode::OK);
    let document: serde_json::Value = json_body(document_response).await;
    assert_eq!(
        declared_status(&document, "/health", "get"),
        Some(200),
        "OpenAPI should declare GET /health 200"
    );
    assert_eq!(
        declared_status(&document, "/session", "get"),
        Some(200),
        "OpenAPI should declare GET /session 200"
    );

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
    assert_eq!(health.status(), StatusCode::OK);

    let sessions = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/session")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("session list response");
    assert_eq!(sessions.status(), StatusCode::OK);

    let unknown = app
        .oneshot(
            Request::builder()
                .uri(format!("/totally-made-up-{}", std::process::id()))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("unknown response");
    assert_eq!(unknown.status(), StatusCode::NOT_FOUND);
}

fn assert_protected_operation(document: &serde_json::Value, path: &str, method: &str) {
    let operation = &document["paths"][path][method];
    assert_eq!(
        operation["security"],
        serde_json::json!([{ "bearerAuth": [] }]),
        "operation {method} {path} should document optional bearer auth"
    );
    assert!(
        operation["responses"]
            .as_object()
            .is_some_and(|responses| responses.contains_key("401")),
        "operation {method} {path} should document auth failures: {operation:?}"
    );
}

fn assert_method(
    paths: &serde_json::Map<String, serde_json::Value>,
    path: &str,
    method: &str,
    success_status: u16,
) {
    let operation = paths
        .get(path)
        .and_then(|path_item| path_item.get(method))
        .unwrap_or_else(|| panic!("missing OpenAPI operation {method} {path}"));
    let responses = operation["responses"]
        .as_object()
        .unwrap_or_else(|| panic!("operation {method} {path} should declare responses"));
    assert!(
        responses.contains_key(&success_status.to_string()),
        "operation {method} {path} should declare success status {success_status}: {operation:?}"
    );
}

fn declared_status(document: &serde_json::Value, path: &str, method: &str) -> Option<u16> {
    document["paths"][path][method]["responses"]
        .as_object()?
        .keys()
        .filter_map(|status| status.parse::<u16>().ok())
        .find(|status| (200..300).contains(status))
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
