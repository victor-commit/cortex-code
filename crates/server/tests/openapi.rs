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
    assert_eq!(
        document["x-cortex-compatibility"]["decision"], "wire-compatible",
        "OpenAPI should expose the selected API compatibility decision"
    );
    assert_eq!(
        document["x-cortex-compatibility"]["recorded_backlog_item"],
        "gap-server-wire-compatibility-decision",
        "OpenAPI should reference the backlog decision item"
    );
    assert_eq!(
        document["x-cortex-compatibility"]["route_contract"]["compatibility_target"],
        "reference-wire",
        "OpenAPI should lock the route compatibility contract"
    );
    assert!(
        document["x-cortex-compatibility"]["route_contract"]["rules"]
            .as_array()
            .is_some_and(|rules| !rules.is_empty()),
        "Route compatibility rules should be non-empty: {document:?}"
    );
    assert!(
        document["x-cortex-compatibility"]["schema_contract"]["rules"]
            .as_array()
            .is_some_and(|rules| !rules.is_empty()),
        "Schema compatibility rules should be non-empty: {document:?}"
    );
    assert!(
        document["x-cortex-compatibility"]["event_contract"]["rules"]
            .as_array()
            .is_some_and(|rules| !rules.is_empty()),
        "Event compatibility rules should be non-empty: {document:?}"
    );
    assert!(
        document["x-cortex-compatibility"]["sdk_contract"]["rules"]
            .as_array()
            .is_some_and(|rules| !rules.is_empty()),
        "SDK compatibility rules should be non-empty: {document:?}"
    );

    let paths = document["paths"].as_object().expect("paths object");
    assert_method(paths, "/health", "get", 200);
    assert_method(paths, "/event", "get", 200);
    assert_method(paths, "/event", "post", 200);
    assert_method(paths, "/session", "get", 200);
    assert_method(paths, "/session", "post", 201);
    assert_method(paths, "/session/{id}", "get", 200);
    assert_method(paths, "/session/{id}", "patch", 200);
    assert_method(paths, "/session/{id}", "delete", 204);
    assert_method(paths, "/session/status", "get", 200);
    assert_method(paths, "/session/{id}/status", "get", 200);
    assert_method(paths, "/session/{id}/wait", "get", 200);
    assert_method(paths, "/session/{id}/context", "get", 200);
    assert_method(paths, "/session/{id}/children", "get", 200);
    assert_method(paths, "/session/{id}/diff", "get", 200);
    assert_method(paths, "/session/{id}/fork", "post", 201);
    assert_method(paths, "/session/{id}/init", "post", 200);
    assert_method(paths, "/session/{id}/summarize", "post", 200);
    assert_method(paths, "/session/{id}/compact", "post", 200);
    assert_method(paths, "/session/{id}/prompt_async", "post", 202);
    assert_method(paths, "/session/{id}/shell", "post", 200);
    assert_method(paths, "/session/{id}/revert", "post", 200);
    assert_method(paths, "/session/{id}/unrevert", "post", 200);
    assert_method(paths, "/session/{id}/message", "get", 200);
    assert_method(paths, "/session/{id}/message", "post", 200);
    assert_method(paths, "/session/{id}/message/{message_id}", "get", 200);
    assert_method(paths, "/session/{id}/message/{message_id}", "delete", 200);
    assert_method(
        paths,
        "/session/{id}/message/{message_id}/part/{part_id}",
        "patch",
        200,
    );
    assert_method(
        paths,
        "/session/{id}/message/{message_id}/part/{part_id}",
        "delete",
        200,
    );
    assert_method(paths, "/session/{id}/todo", "get", 200);
    assert_method(paths, "/session/{id}/todos", "get", 200);
    assert_method(paths, "/session/{id}/prompt", "get", 200);
    assert_method(paths, "/session/{id}/prompt/append", "post", 200);
    assert_method(paths, "/session/{id}/prompt/submit", "post", 200);
    assert_method(paths, "/session/{id}/prompt/clear", "post", 200);
    assert_method(paths, "/session/{id}/command", "post", 200);
    assert_method(paths, "/permission/pending", "get", 200);
    assert_method(paths, "/permission/reply", "post", 200);
    assert_method(paths, "/session/{id}/permission/pending", "get", 200);
    assert_method(paths, "/session/{id}/permission/reply", "post", 200);
    assert_method(paths, "/session/{id}/abort", "post", 200);
    assert_method(paths, "/session/{id}/share", "post", 200);
    assert_method(paths, "/session/{id}/unshare", "post", 200);
    assert_method(paths, "/share/{share_id}", "get", 200);
    assert_method(paths, "/project/discovery", "get", 200);
    assert_method(paths, "/experimental/workspace/adapter", "get", 200);
    assert_method(paths, "/config", "get", 200);
    assert_method(paths, "/config", "patch", 200);
    assert_method(paths, "/config/providers", "get", 200);
    assert_method(paths, "/provider", "get", 200);
    assert_method(paths, "/auth", "get", 200);
    assert_method(paths, "/auth/callback", "get", 200);
    assert_method(paths, "/auth/{provider}", "put", 200);
    assert_method(paths, "/auth/{provider}", "delete", 200);
    assert_method(paths, "/auth/{provider}/authorize", "post", 200);
    assert_method(paths, "/file", "get", 200);
    assert_method(paths, "/file/content", "get", 200);
    assert_method(paths, "/file/status", "get", 200);
    assert_method(paths, "/find", "get", 200);
    assert_method(paths, "/find/file", "get", 200);
    assert_method(paths, "/find/symbol", "get", 200);
    assert_method(paths, "/agent", "get", 200);
    assert_method(paths, "/model", "get", 200);
    assert_method(paths, "/command", "get", 200);
    assert_method(paths, "/skill", "get", 200);
    assert_eq!(
        document["components"]["securitySchemes"]["bearerAuth"]["scheme"], "bearer",
        "OpenAPI should document the Authorization bearer scheme"
    );
    assert_protected_operation(&document, "/session", "get");
    assert_protected_operation(&document, "/session/{id}/message", "post");
    assert_protected_operation(&document, "/event", "get");
    assert_protected_operation(&document, "/permission/pending", "get");
    assert_protected_operation(&document, "/file/content", "get");
    assert_protected_operation(&document, "/find", "get");
    assert_protected_operation(&document, "/agent", "get");
    assert_eq!(
        document["components"]["schemas"]["MessageRequest"]["properties"]["model"]["type"],
        "string",
        "message requests should document the per-turn model selector"
    );
    assert!(
        document["components"]["schemas"]["MessageRequest"]["properties"]["model"]["description"]
            .as_str()
            .is_some_and(|description| {
                description.contains("provider/model")
                    && description.contains("google/gemini-2.5-flash")
            }),
        "model schema should explain the fully-qualified id shape and default: {document:?}"
    );
    assert_eq!(
        document["components"]["schemas"]["PromptResponse"]["properties"]["agent"]["type"],
        "string",
        "prompt responses should document the active agent field"
    );
    assert_eq!(
        document["components"]["schemas"]["WorkspaceAdapterDiscovery"]["required"],
        serde_json::json!(["plugin", "type", "name", "description"]),
        "workspace adapter discovery entries should expose stable plugin/type/name/description fields"
    );
    assert_eq!(
        document["components"]["schemas"]["ProjectDiscoveryResponse"]["properties"]["workspace_adapters"]
            ["items"]["$ref"],
        "#/components/schemas/WorkspaceAdapterDiscovery",
        "project discovery should document workspace adapter entries"
    );
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
                    | "/session/status"
                    | "/session/{id}"
                    | "/session/{id}/status"
                    | "/session/{id}/wait"
                    | "/session/{id}/context"
                    | "/session/{id}/children"
                    | "/session/{id}/diff"
                    | "/session/{id}/fork"
                    | "/session/{id}/init"
                    | "/session/{id}/summarize"
                    | "/session/{id}/compact"
                    | "/session/{id}/prompt_async"
                    | "/session/{id}/shell"
                    | "/session/{id}/revert"
                    | "/session/{id}/unrevert"
                    | "/session/{id}/message"
                    | "/session/{id}/message/{message_id}"
                    | "/session/{id}/message/{message_id}/part/{part_id}"
                    | "/session/{id}/todo"
                    | "/session/{id}/todos"
                    | "/session/{id}/prompt"
                    | "/session/{id}/prompt/append"
                    | "/session/{id}/prompt/submit"
                    | "/session/{id}/prompt/clear"
                    | "/session/{id}/command"
                    | "/permission/pending"
                    | "/permission/reply"
                    | "/session/{id}/permission/pending"
                    | "/session/{id}/permission/reply"
                    | "/session/{id}/abort"
                    | "/session/{id}/share"
                    | "/session/{id}/unshare"
                    | "/share/{share_id}"
                    | "/project/discovery"
                    | "/experimental/workspace/adapter"
                    | "/config"
                    | "/config/providers"
                    | "/provider"
                    | "/auth"
                    | "/auth/callback"
                    | "/auth/{provider}"
                    | "/auth/{provider}/authorize"
                    | "/file"
                    | "/file/content"
                    | "/file/status"
                    | "/find"
                    | "/find/file"
                    | "/find/symbol"
                    | "/agent"
                    | "/model"
                    | "/command"
                    | "/skill"
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
    assert_eq!(
        declared_status(&document, "/project/discovery", "get"),
        Some(200),
        "OpenAPI should declare GET /project/discovery 200"
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
    let bytes = to_bytes(response.into_body(), 524_288).await.expect("body");
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
