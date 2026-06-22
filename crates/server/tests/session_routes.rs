use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode, header};
use db::Session;
use tower::ServiceExt;

static SCRATCH_COUNTER: AtomicU64 = AtomicU64::new(0);

#[tokio::test]
async fn session_crud_routes_create_list_get_and_delete_with_database_backing() {
    let scratch = Scratch::new("server-session-crud");
    let app = server::app_with_db_path(scratch.db_path());

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/session")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from("{}"))
                .expect("request"),
        )
        .await
        .expect("create response");
    assert_eq!(response.status(), StatusCode::CREATED);
    assert_json_content_type(&response);
    let created: Session = json_body(response).await;
    assert!(!created.id.is_empty(), "session id must be non-empty");

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/session")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("list response");
    assert_eq!(response.status(), StatusCode::OK);
    assert_json_content_type(&response);
    let listed: Vec<Session> = json_body(response).await;
    assert!(
        listed.iter().any(|session| session.id == created.id),
        "freshly created session should appear in list: {listed:?}"
    );

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/session/{}", created.id))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("get response");
    assert_eq!(response.status(), StatusCode::OK);
    assert_json_content_type(&response);
    let fetched: Session = json_body(response).await;
    assert_eq!(fetched.id, created.id);

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!("/session/{}", created.id))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("delete response");
    assert_eq!(response.status(), StatusCode::NO_CONTENT);

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/session/{}", created.id))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("deleted get response");
    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/session")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("post-delete list response");
    assert_eq!(response.status(), StatusCode::OK);
    let listed_after_delete: Vec<Session> = json_body(response).await;
    assert!(
        listed_after_delete
            .iter()
            .all(|session| session.id != created.id),
        "deleted session should be absent from list: {listed_after_delete:?}"
    );

    let db = db::Database::open(scratch.db_path()).expect("reopen db");
    assert!(
        db.sessions()
            .get(&created.id)
            .expect("query deleted session")
            .is_none(),
        "deleted session should be absent from the persisted database"
    );
}

#[tokio::test]
async fn unknown_session_id_returns_not_found_for_get_and_delete() {
    let scratch = Scratch::new("server-session-unknown");
    let app = server::app_with_db_path(scratch.db_path());
    let unknown_id = format!("does-not-exist-{}", std::process::id());

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/session/{unknown_id}"))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("unknown get response");
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert_json_content_type(&response);
    let body: serde_json::Value = json_body(response).await;
    assert!(
        body.get("error")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|message| message.contains(&unknown_id)),
        "not-found body should name the unknown id: {body:?}"
    );

    let response = app
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!("/session/{unknown_id}"))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("unknown delete response");
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert_json_content_type(&response);
    let body: serde_json::Value = json_body(response).await;
    assert!(
        body.get("error")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|message| message.contains(&unknown_id)),
        "not-found body should name the unknown id: {body:?}"
    );
}

#[tokio::test]
async fn malformed_create_bodies_are_client_errors_and_health_remains_ok() {
    let scratch = Scratch::new("server-session-malformed-create");
    let app = server::app_with_db_path(scratch.db_path());

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/session")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from("{not json"))
                .expect("request"),
        )
        .await
        .expect("malformed JSON response");
    assert!(
        response.status().is_client_error(),
        "malformed JSON should be a client error, got {}",
        response.status()
    );

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/session")
                .header(header::CONTENT_TYPE, "text/plain")
                .body(Body::from("{}"))
                .expect("request"),
        )
        .await
        .expect("wrong content-type response");
    assert!(
        response.status().is_client_error(),
        "wrong content type should be a client error, got {}",
        response.status()
    );

    let response = app
        .oneshot(
            Request::builder()
                .uri("/health")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("health response");
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value = json_body(response).await;
    assert_eq!(body, serde_json::json!({"status": "ok"}));
}

#[tokio::test]
async fn json_returning_session_routes_set_json_content_type() {
    let scratch = Scratch::new("server-session-content-type");
    let app = server::app_with_db_path(scratch.db_path());

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/session")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from("{}"))
                .expect("request"),
        )
        .await
        .expect("create response");
    assert_eq!(response.status(), StatusCode::CREATED);
    assert_json_content_type(&response);
    let created: Session = json_body(response).await;

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/session")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("list response");
    assert_eq!(response.status(), StatusCode::OK);
    assert_json_content_type(&response);

    let response = app
        .oneshot(
            Request::builder()
                .uri(format!("/session/{}", created.id))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("get response");
    assert_eq!(response.status(), StatusCode::OK);
    assert_json_content_type(&response);
}

async fn json_body<T>(response: axum::response::Response) -> T
where
    T: serde::de::DeserializeOwned,
{
    let bytes = to_bytes(response.into_body(), 8192).await.expect("body");
    serde_json::from_slice(&bytes).expect("JSON body")
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
