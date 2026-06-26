use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode, header};
use db::Session;
use futures_util::StreamExt;
use futures_util::future::BoxFuture;
use llm::{FinishReason, LlmEvent, LlmRequest, LlmStream, Provider, Usage};
use tower::ServiceExt;

static SCRATCH_COUNTER: AtomicU64 = AtomicU64::new(0);

#[tokio::test]
async fn server_response_sse_and_audit_redact_and_attribute_security_events() {
    let scratch = Scratch::new("server-security");
    let secret = fake_secret();
    let app = server::app_with_state(
        server::ServerState::with_provider(
            scratch.db_path(),
            Arc::new(SecretProvider {
                secret: secret.clone(),
            }),
        )
        .with_cwd(scratch.project_path())
        .with_audit_path(scratch.audit_path()),
    );
    let session = create_session(&app).await;
    let mut stream = subscribe(&app).await;

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/session/{}/message", session.id))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"text":"repeat the secret"}"#))
                .expect("request"),
        )
        .await
        .expect("message response");
    assert_eq!(response.status(), StatusCode::OK);
    let body_bytes = to_bytes(response.into_body(), 8192).await.expect("body");
    let body_text = String::from_utf8(body_bytes.to_vec()).expect("response utf8");
    assert!(
        !body_text.contains(&secret),
        "server response leaked: {body_text}"
    );
    assert!(
        body_text.contains(redaction::PLACEHOLDER),
        "server response should carry placeholder: {body_text}"
    );

    let events = collect_data_events(&mut stream, 2, Duration::from_secs(3)).await;
    let events_text = serde_json::to_string(&events).expect("events json");
    assert!(!events_text.contains(&secret), "SSE leaked: {events_text}");
    assert!(
        events_text.contains(redaction::PLACEHOLDER),
        "SSE should carry placeholder: {events_text}"
    );

    let command = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/session/{}/command", session.id))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"command":"echo server-audit-marker"}"#))
                .expect("request"),
        )
        .await
        .expect("command response");
    assert_eq!(command.status(), StatusCode::OK);

    let records = audit::read_jsonl(&scratch.audit_path()).expect("audit records");
    assert!(records.iter().any(|record| {
        record["type"] == "shell_execution"
            && record["surface"] == "serve"
            && record["source"] == "server"
            && record["session"] == session.id
            && record["command"] == "echo server-audit-marker"
    }));
    let audit_text = std::fs::read_to_string(scratch.audit_path()).expect("audit text");
    assert!(!audit_text.contains(&secret), "audit leaked: {audit_text}");
}

async fn subscribe(app: &axum::Router) -> axum::body::BodyDataStream {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/event")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("event response");
    assert_eq!(response.status(), StatusCode::OK);
    response.into_body().into_data_stream()
}

async fn collect_data_events(
    stream: &mut axum::body::BodyDataStream,
    min_events: usize,
    timeout: Duration,
) -> Vec<serde_json::Value> {
    let start = Instant::now();
    let mut buffer = String::new();
    let mut events = Vec::new();
    while events.len() < min_events && start.elapsed() < timeout {
        let remaining = timeout.saturating_sub(start.elapsed());
        let Ok(Some(chunk)) = tokio::time::timeout(remaining, stream.next()).await else {
            break;
        };
        let chunk = chunk.expect("SSE chunk");
        buffer.push_str(std::str::from_utf8(&chunk).expect("utf8 SSE"));
        events = data_values(&buffer)
            .iter()
            .map(|line| serde_json::from_str(line).expect("JSON data frame"))
            .collect();
    }
    events
}

fn data_values(frame_text: &str) -> Vec<String> {
    frame_text
        .lines()
        .filter_map(|line| line.strip_prefix("data:"))
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

async fn create_session(app: &axum::Router) -> Session {
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
    let bytes = to_bytes(response.into_body(), 8192).await.expect("body");
    serde_json::from_slice(&bytes).expect("session JSON")
}

#[derive(Debug)]
struct SecretProvider {
    secret: String,
}

impl Provider for SecretProvider {
    fn stream<'a>(
        &'a self,
        _request: LlmRequest,
    ) -> BoxFuture<'a, Result<LlmStream, llm::LlmError>> {
        Box::pin(async move {
            let events = vec![
                LlmEvent::TextDelta {
                    text: format!("server secret {}", self.secret),
                },
                LlmEvent::Usage(Usage {
                    input_tokens: 1,
                    output_tokens: 1,
                    total_tokens: 2,
                    cache_creation_input_tokens: None,
                    cache_read_input_tokens: None,
                    reasoning_output_tokens: None,
                    cost: None,
                }),
                LlmEvent::Done {
                    reason: FinishReason::Stop,
                },
            ];
            Ok(Box::pin(futures_util::stream::iter(events)) as LlmStream)
        })
    }
}

struct Scratch {
    dir: PathBuf,
}

impl Scratch {
    fn new(label: &str) -> Self {
        let id = SCRATCH_COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("cortex-{label}-{}-{id}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("project")).expect("create scratch dir");
        Self { dir }
    }

    fn db_path(&self) -> PathBuf {
        self.dir.join("cortex.db")
    }

    fn project_path(&self) -> PathBuf {
        self.dir.join("project")
    }

    fn audit_path(&self) -> PathBuf {
        self.dir.join("audit.jsonl")
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn fake_secret() -> String {
    format!("sk-{}-{}", "or-v1", "c0ffee00".repeat(8))
}
