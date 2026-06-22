use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use axum::body::{Body, Bytes, to_bytes};
use axum::http::{Request, StatusCode, header};
use db::Session;
use events::{Event, EventBus};
use futures_util::StreamExt;
use futures_util::future::BoxFuture;
use llm::{FinishReason, LlmEvent, LlmRequest, LlmStream, Provider, Role, ToolCall, Usage};
use tower::ServiceExt;

static SCRATCH_COUNTER: AtomicU64 = AtomicU64::new(0);

#[tokio::test]
async fn event_route_streams_json_sse_frames_from_the_bus() {
    let scratch = Scratch::new("server-event-route");
    let bus = Arc::new(EventBus::new());
    let app = server::app_with_state(
        server::ServerState::new(scratch.db_path()).with_event_bus(bus.clone()),
    );

    let response = app
        .oneshot(
            Request::builder()
                .uri("/event")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("event response");

    assert_eq!(response.status(), StatusCode::OK);
    assert_sse_content_type(&response);

    let mut stream = response.into_body().into_data_stream();
    bus.publish(&Event::assistant_text_delta(
        "ses_test", "msg_test", "hello",
    ));

    let bytes = next_chunk(&mut stream).await;
    let text = String::from_utf8(bytes.to_vec()).expect("utf8 sse frame");
    let data = data_values(&text);
    assert_eq!(data.len(), 1, "one data line in frame: {text:?}");
    let parsed: serde_json::Value = serde_json::from_str(&data[0]).expect("data JSON");
    assert_eq!(parsed["kind"], "assistant.text_delta");
    assert_eq!(parsed["session_id"], "ses_test");
    assert_eq!(parsed["message_id"], "msg_test");
    assert_eq!(parsed["text"], "hello");
}

#[tokio::test]
async fn prompt_turn_fans_out_correlated_text_deltas_to_multiple_subscribers() {
    let scratch = Scratch::new("server-event-fanout");
    let bus = Arc::new(EventBus::new());
    let provider = Arc::new(StreamingEchoProvider);
    let state = server::ServerState::with_provider(scratch.db_path(), provider).with_event_bus(bus);
    let app = server::app_with_state(state);
    let session = create_session(&app).await;

    let mut first = subscribe(&app).await;
    let mut second = subscribe(&app).await;

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/session/{}/message", session.id))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"text":"count please"}"#))
                .expect("request"),
        )
        .await
        .expect("message response");
    assert_eq!(response.status(), StatusCode::OK);

    let first_events = collect_data_events(&mut first, 4, Duration::from_secs(3)).await;
    let second_events = collect_data_events(&mut second, 4, Duration::from_secs(3)).await;
    for events in [&first_events, &second_events] {
        let deltas: Vec<&serde_json::Value> = events
            .iter()
            .filter(|event| event["kind"] == "assistant.text_delta")
            .collect();
        assert!(
            deltas.len() >= 2,
            "subscriber should receive multiple text deltas: {events:?}"
        );
        assert!(
            deltas.iter().all(|event| event["session_id"] == session.id),
            "all deltas carry the originating session id: {events:?}"
        );
        let reconstructed = deltas
            .iter()
            .filter_map(|event| event["text"].as_str())
            .collect::<String>();
        assert!(
            reconstructed.contains("one two three"),
            "deltas reconstruct the answer: {reconstructed:?}"
        );
    }
}

#[tokio::test]
async fn tool_using_turn_surfaces_correlated_tool_lifecycle_events() {
    let scratch = Scratch::new("server-event-tool");
    let bus = Arc::new(EventBus::new());
    let provider = Arc::new(ToolCallingProvider::default());
    let state = server::ServerState::with_provider(scratch.db_path(), provider).with_event_bus(bus);
    let app = server::app_with_state(state);
    let session = create_session(&app).await;
    let mut stream = subscribe(&app).await;

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/session/{}/message", session.id))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"text":"use bash"}"#))
                .expect("request"),
        )
        .await
        .expect("message response");
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value = json_body(response).await;
    assert!(
        body["content"]
            .as_str()
            .is_some_and(|text| text.contains("cortex-sse-tool-marker")),
        "final assistant text should include the observable tool marker: {body:?}"
    );

    let events = collect_data_events(&mut stream, 6, Duration::from_secs(3)).await;
    assert!(
        events.iter().any(|event| event["kind"] == "tool.call"
            && event["name"] == "bash"
            && event["session_id"] == session.id),
        "SSE should include a correlated bash tool-call event: {events:?}"
    );
    assert!(
        events.iter().any(|event| event["kind"] == "tool.result"
            && event["name"] == "bash"
            && event["session_id"] == session.id),
        "SSE should include a correlated bash tool-result event: {events:?}"
    );
}

fn assert_sse_content_type(response: &axum::response::Response) {
    let content_type = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("");
    assert!(
        content_type.starts_with("text/event-stream"),
        "expected text/event-stream content type, got {content_type:?}"
    );
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
    assert_sse_content_type(&response);
    response.into_body().into_data_stream()
}

async fn next_chunk(stream: &mut axum::body::BodyDataStream) -> Bytes {
    tokio::time::timeout(Duration::from_secs(3), stream.next())
        .await
        .expect("next SSE chunk before timeout")
        .expect("SSE stream should stay open")
        .expect("SSE chunk should be ok")
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
        let values = data_values(&buffer);
        events = values
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

async fn json_body<T>(response: axum::response::Response) -> T
where
    T: serde::de::DeserializeOwned,
{
    let bytes = to_bytes(response.into_body(), 8192).await.expect("body");
    serde_json::from_slice(&bytes).expect("JSON body")
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
    json_body(response).await
}

#[derive(Debug, Default)]
struct StreamingEchoProvider;

impl Provider for StreamingEchoProvider {
    fn stream<'a>(
        &'a self,
        _request: LlmRequest,
    ) -> BoxFuture<'a, Result<LlmStream, llm::LlmError>> {
        Box::pin(async {
            let events = vec![
                LlmEvent::TextDelta {
                    text: "one ".to_owned(),
                },
                LlmEvent::TextDelta {
                    text: "two ".to_owned(),
                },
                LlmEvent::TextDelta {
                    text: "three".to_owned(),
                },
                LlmEvent::Usage(Usage {
                    input_tokens: 3,
                    output_tokens: 3,
                    total_tokens: 6,
                    cost: Some(0.000_001),
                }),
                LlmEvent::Done {
                    reason: FinishReason::Stop,
                },
            ];
            Ok(Box::pin(futures_util::stream::iter(events)) as LlmStream)
        })
    }
}

#[derive(Debug, Default)]
struct ToolCallingProvider {
    calls: AtomicUsize,
}

impl Provider for ToolCallingProvider {
    fn stream<'a>(
        &'a self,
        request: LlmRequest,
    ) -> BoxFuture<'a, Result<LlmStream, llm::LlmError>> {
        Box::pin(async move {
            let call = self.calls.fetch_add(1, Ordering::SeqCst);
            let has_tool_result = request
                .messages
                .iter()
                .any(|message| message.role == Role::Tool);
            let events = if call == 0 && !has_tool_result {
                vec![
                    LlmEvent::ToolCall(ToolCall {
                        id: "call_bash".to_owned(),
                        name: "bash".to_owned(),
                        arguments: r#"{"command":"echo cortex-sse-tool-marker"}"#.to_owned(),
                    }),
                    LlmEvent::Done {
                        reason: FinishReason::ToolCalls,
                    },
                ]
            } else {
                vec![
                    LlmEvent::TextDelta {
                        text: "tool output: cortex-sse-tool-marker".to_owned(),
                    },
                    LlmEvent::Usage(Usage {
                        input_tokens: 5,
                        output_tokens: 5,
                        total_tokens: 10,
                        cost: Some(0.000_002),
                    }),
                    LlmEvent::Done {
                        reason: FinishReason::Stop,
                    },
                ]
            };
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
