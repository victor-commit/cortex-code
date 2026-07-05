use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use axum::body::{Body, to_bytes};
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

    let events = collect_until_kind(
        &mut stream,
        "session.next.text.delta",
        Duration::from_secs(3),
    )
    .await;
    let legacy = events
        .iter()
        .find(|event| event["kind"] == "assistant.text_delta")
        .expect("legacy assistant.text_delta frame present");
    assert_eq!(legacy["session_id"], "ses_test");
    assert_eq!(legacy["message_id"], "msg_test");
    assert_eq!(legacy["text"], "hello");

    let compat = events
        .iter()
        .find(|event| event["kind"] == "session.next.text.delta")
        .expect("compatible session.next.text.delta frame present");
    assert_eq!(compat["session_id"], "ses_test");
    assert_eq!(compat["message_id"], "msg_test");
    assert_eq!(compat["text"], "hello");

    assert!(
        events
            .iter()
            .any(|event| event["kind"] == "session.next.text.start"),
        "the granular text channel start marker precedes its deltas: {events:?}"
    );
}

#[tokio::test]
async fn event_route_streams_mcp_catalogue_change_events() {
    let scratch = Scratch::new("server-event-mcp-catalogue");
    let bus = Arc::new(EventBus::new());
    let app = server::app_with_state(
        server::ServerState::new(scratch.db_path()).with_event_bus(bus.clone()),
    );
    let mut stream = subscribe(&app).await;

    bus.publish(&Event::mcp_resources_changed(
        "demo",
        vec!["notes://demo".to_owned()],
    ));
    bus.publish(&Event::mcp_prompts_changed(
        "demo",
        vec!["greet".to_owned()],
    ));

    let events = collect_data_events(&mut stream, 2, Duration::from_secs(3)).await;
    assert!(
        events
            .iter()
            .any(|event| event["kind"] == "mcp.resources.changed"
                && event["server"] == "demo"
                && event["resources"]
                    .as_array()
                    .is_some_and(|entries| entries.iter().any(|entry| entry == "notes://demo"))),
        "SSE should include mcp.resources.changed payloads: {events:?}"
    );
    assert!(
        events
            .iter()
            .any(|event| event["kind"] == "mcp.prompts.changed"
                && event["server"] == "demo"
                && event["prompts"]
                    .as_array()
                    .is_some_and(|entries| entries.iter().any(|entry| entry == "greet"))),
        "SSE should include mcp.prompts.changed payloads: {events:?}"
    );
}

#[tokio::test]
async fn event_route_redacts_split_text_and_reasoning_deltas_before_sse_delivery() {
    let scratch = Scratch::new("server-event-redaction");
    let bus = Arc::new(EventBus::new());
    let app = server::app_with_state(
        server::ServerState::new(scratch.db_path()).with_event_bus(bus.clone()),
    );
    let mut stream = subscribe(&app).await;
    let body = "c0ffee00".repeat(8);
    let secret = format!("sk-{}-{}", "or-v1", body);

    for fragment in [
        "text before ",
        "sk-",
        "or-v1-",
        &body[..16],
        &body[16..],
        " text after",
    ] {
        bus.publish(&Event::assistant_text_delta(
            "ses_split",
            "msg_split",
            fragment,
        ));
    }
    for fragment in [
        "reason before ",
        "sk-",
        "or-v1-",
        &body[..24],
        &body[24..],
        " reason after",
    ] {
        bus.publish(&Event::assistant_reasoning_delta(
            "ses_split",
            "msg_split",
            fragment,
        ));
    }
    bus.publish(&Event::turn_done("ses_split", "msg_split", "stop", 1));

    let events = collect_until_kind(&mut stream, "turn.done", Duration::from_secs(3)).await;
    let delivered = serde_json::to_string(&events).expect("events JSON");
    assert!(
        !delivered.contains(&secret),
        "SSE frames leaked split secret: {delivered}"
    );
    assert!(
        delivered.contains(redaction::PLACEHOLDER),
        "SSE frames should include canonical placeholder: {delivered}"
    );

    let text = events
        .iter()
        .filter(|event| event["kind"] == "assistant.text_delta")
        .filter_map(|event| event["text"].as_str())
        .collect::<String>();
    assert_eq!(
        text,
        format!("text before {} text after", redaction::PLACEHOLDER)
    );
    // The compatible granular vocabulary reconstructs the same redacted text.
    let next_text = events
        .iter()
        .filter(|event| event["kind"] == "session.next.text.delta")
        .filter_map(|event| event["text"].as_str())
        .collect::<String>();
    assert_eq!(next_text, text, "granular text deltas match legacy deltas");
    let reasoning = events
        .iter()
        .filter(|event| event["kind"] == "assistant.reasoning_delta")
        .filter_map(|event| event["text"].as_str())
        .collect::<String>();
    assert_eq!(
        reasoning,
        format!("reason before {} reason after", redaction::PLACEHOLDER)
    );
    assert!(
        events.iter().any(|event| event["kind"] == "turn.done"),
        "terminal event is still delivered after redactor flush: {events:?}"
    );
    assert!(
        events
            .iter()
            .any(|event| event["kind"] == "session.next.text.end"),
        "granular text channel emits an end marker after flush: {events:?}"
    );
}

#[tokio::test]
async fn event_route_redacts_mcp_log_payloads_before_sse_delivery() {
    let scratch = Scratch::new("server-event-mcp-log-redaction");
    let bus = Arc::new(EventBus::new());
    let app = server::app_with_state(
        server::ServerState::new(scratch.db_path()).with_event_bus(bus.clone()),
    );
    let mut stream = subscribe(&app).await;
    let secret = format!("sk-{}-{}", "or-v1", "deadbeef".repeat(8));

    bus.publish(&Event::mcp_log_message(
        "demo",
        "warning",
        Some("fixture".to_owned()),
        serde_json::json!({
            "message": "server warning",
            "token": secret,
        }),
    ));

    let events = collect_data_events(&mut stream, 1, Duration::from_secs(3)).await;
    let payload = serde_json::to_string(&events).expect("events JSON");
    assert!(
        !payload.contains("deadbeef"),
        "SSE mcp.log payload must be redacted: {payload}"
    );
    assert!(
        payload.contains(redaction::PLACEHOLDER),
        "redacted mcp.log payload should include placeholder: {payload}"
    );
    assert!(
        events.iter().any(|event| event["kind"] == "mcp.log"
            && event["server"] == "demo"
            && event["level"] == "warning"),
        "SSE should include the mcp.log event metadata: {events:?}"
    );
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

    let first_events = collect_until_kind(
        &mut first,
        "session.next.step.finished",
        Duration::from_secs(5),
    )
    .await;
    let second_events = collect_until_kind(
        &mut second,
        "session.next.step.finished",
        Duration::from_secs(5),
    )
    .await;
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

    // VAL-GAP-SSE-006: both subscribers observe an identical, ordered event kind sequence with
    // exactly one terminal event.
    let first_kinds: Vec<&str> = first_events
        .iter()
        .filter_map(|event| event["kind"].as_str())
        .collect();
    let second_kinds: Vec<&str> = second_events
        .iter()
        .filter_map(|event| event["kind"].as_str())
        .collect();
    assert_eq!(
        first_kinds, second_kinds,
        "fan-out subscribers receive identical ordered event sequences"
    );
    assert_eq!(
        first_kinds
            .iter()
            .filter(|kind| **kind == "turn.done")
            .count(),
        1,
        "exactly one terminal turn.done event is delivered: {first_kinds:?}"
    );
    assert_eq!(
        first_kinds
            .iter()
            .filter(|kind| **kind == "session.idle")
            .count(),
        1,
        "exactly one terminal session.idle event is delivered: {first_kinds:?}"
    );
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

    let events = collect_until_kind(
        &mut stream,
        "session.next.step.finished",
        Duration::from_secs(5),
    )
    .await;
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
    assert!(
        events
            .iter()
            .any(|event| event["kind"] == "session.next.tool.started"
                && event["name"] == "bash"
                && event["session_id"] == session.id),
        "SSE should include a granular session.next.tool.started event: {events:?}"
    );
    assert!(
        events
            .iter()
            .any(|event| event["kind"] == "session.next.tool.ended"
                && event["name"] == "bash"
                && event["session_id"] == session.id),
        "SSE should include a granular session.next.tool.ended event: {events:?}"
    );
    assert!(
        events.iter().any(|event| event["kind"] == "message.parts"
            && event["session_id"] == session.id
            && event["parts"]
                .as_array()
                .is_some_and(|parts| !parts.is_empty())),
        "SSE should expose persisted rich message parts: {events:?}"
    );
}

#[tokio::test]
async fn command_route_surfaces_correlated_tool_lifecycle_events() {
    let scratch = Scratch::new("server-event-command");
    let bus = Arc::new(EventBus::new());
    let state = server::ServerState::new(scratch.db_path()).with_event_bus(bus);
    let app = server::app_with_state(state);
    let session = create_session(&app).await;
    let mut stream = subscribe(&app).await;

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/session/{}/command", session.id))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    r#"{"command":"echo cortex-event-command-marker"}"#,
                ))
                .expect("request"),
        )
        .await
        .expect("command response");
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value = json_body(response).await;
    assert!(
        body["output"]
            .as_str()
            .is_some_and(|output| output.contains("cortex-event-command-marker")),
        "command response should include stdout marker: {body:?}"
    );

    let events = collect_until_kind(&mut stream, "command.executed", Duration::from_secs(3)).await;
    assert!(
        events.iter().any(|event| event["kind"] == "tool.call"
            && event["name"] == "bash"
            && event["session_id"] == session.id),
        "SSE should include a correlated bash tool-call event: {events:?}"
    );
    assert!(
        events.iter().any(|event| event["kind"] == "tool.result"
            && event["name"] == "bash"
            && event["session_id"] == session.id
            && event["is_error"] == false),
        "SSE should include a correlated bash tool-result event: {events:?}"
    );
    assert!(
        events
            .iter()
            .any(|event| event["kind"] == "command.executed" && event["session_id"] == session.id),
        "SSE should include a command.executed subsystem event: {events:?}"
    );
}

#[tokio::test]
async fn posted_tui_turn_events_are_rebroadcast_and_abort_observes_remote_activity() {
    let scratch = Scratch::new("server-event-posted-tui");
    let bus = Arc::new(EventBus::new());
    let state = server::ServerState::new(scratch.db_path()).with_event_bus(bus);
    let app = server::app_with_state(state);
    let session = create_session(&app).await;
    let mut stream = subscribe(&app).await;
    let message_id = "tui-msg-remote-1";

    post_event(
        &app,
        &Event::turn_started(session.id.clone(), message_id.to_owned()),
    )
    .await;

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/session/{}/abort", session.id))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("abort response");
    assert_eq!(response.status(), StatusCode::OK);
    let abort: serde_json::Value = json_body(response).await;
    assert_eq!(abort["status"], "abort_requested");
    assert_eq!(
        abort["active"], true,
        "a posted TUI turn.started event should make server abort observe remote activity"
    );

    post_event(
        &app,
        &Event::turn_usage(
            session.id.clone(),
            message_id.to_owned(),
            3,
            4,
            7,
            None,
            None,
            Some(0.1),
        ),
    )
    .await;
    post_event(
        &app,
        &Event::turn_done(session.id.clone(), message_id.to_owned(), "stop", 1),
    )
    .await;

    let events = collect_until_kind(&mut stream, "turn.done", Duration::from_secs(3)).await;
    assert!(
        events.iter().any(|event| event["kind"] == "turn.started"
            && event["session_id"] == session.id
            && event["message_id"] == message_id),
        "posted turn.started should be visible on SSE: {events:?}"
    );
    assert!(
        events.iter().any(|event| event["kind"] == "turn.usage"
            && event["session_id"] == session.id
            && event["total_tokens"] == 7),
        "posted turn.usage should be visible on SSE: {events:?}"
    );
    assert!(
        events.iter().any(|event| event["kind"] == "turn.done"
            && event["session_id"] == session.id
            && event["reason"] == "stop"),
        "posted turn.done should be visible on SSE: {events:?}"
    );
}

// VAL-GAP-SSE-001: message and part updates use stable, client-consumable event names whose
// payload IDs match the REST session/message IDs.
#[tokio::test]
async fn message_and_part_updates_carry_stable_names_and_rest_correlated_ids() {
    let scratch = Scratch::new("server-event-message-updates");
    let bus = Arc::new(EventBus::new());
    let provider = Arc::new(StreamingEchoProvider);
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
                .body(Body::from(r#"{"text":"count please"}"#))
                .expect("request"),
        )
        .await
        .expect("message response");
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value = json_body(response).await;
    let assistant_id = body["message"]["id"]
        .as_str()
        .expect("assistant message id in REST response")
        .to_owned();

    let events = collect_until_kind(
        &mut stream,
        "session.next.step.finished",
        Duration::from_secs(5),
    )
    .await;

    let user_updated = events
        .iter()
        .find(|event| event["kind"] == "message.updated" && event["role"] == "user")
        .expect("user message.updated event");
    assert_eq!(user_updated["session_id"], session.id);

    let assistant_updated = events
        .iter()
        .find(|event| event["kind"] == "message.updated" && event["role"] == "assistant")
        .expect("assistant message.updated event");
    assert_eq!(assistant_updated["session_id"], session.id);
    assert_eq!(
        assistant_updated["message_id"], assistant_id,
        "message.updated id matches the REST assistant message id"
    );

    let part_updated = events
        .iter()
        .find(|event| {
            event["kind"] == "message.part.updated" && event["message_id"] == assistant_id
        })
        .expect("message.part.updated event for the assistant message");
    assert_eq!(part_updated["session_id"], session.id);
    assert!(
        part_updated["part"].is_object(),
        "message.part.updated carries the typed part payload: {part_updated:?}"
    );
}

// VAL-GAP-SSE-002: streaming emits granular text start/delta/end (and tool start/end) so a client
// can reconstruct a full turn from SSE; reconstructed text equals the final message.
#[tokio::test]
async fn granular_streaming_reconstructs_the_turn_without_polling() {
    let scratch = Scratch::new("server-event-granular");
    let bus = Arc::new(EventBus::new());
    let provider = Arc::new(StreamingEchoProvider);
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
                .body(Body::from(r#"{"text":"count please"}"#))
                .expect("request"),
        )
        .await
        .expect("message response");
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value = json_body(response).await;
    let final_text = body["content"].as_str().expect("final content").to_owned();

    let events =
        collect_until_kind(&mut stream, "session.next.text.end", Duration::from_secs(5)).await;
    let kinds: Vec<&str> = events
        .iter()
        .filter_map(|event| event["kind"].as_str())
        .collect();

    let start = kinds
        .iter()
        .position(|kind| *kind == "session.next.text.start")
        .expect("text start present");
    let first_delta = kinds
        .iter()
        .position(|kind| *kind == "session.next.text.delta")
        .expect("text delta present");
    let end = kinds
        .iter()
        .position(|kind| *kind == "session.next.text.end")
        .expect("text end present");
    assert!(
        start < first_delta && first_delta < end,
        "granular text channel is ordered start < delta < end: {kinds:?}"
    );

    let reconstructed = events
        .iter()
        .filter(|event| event["kind"] == "session.next.text.delta")
        .filter_map(|event| event["text"].as_str())
        .collect::<String>();
    assert_eq!(
        reconstructed, final_text,
        "granular deltas reconstruct the final message text"
    );
}

// VAL-GAP-SSE-003: step, model, agent, and context transitions are emitted per turn; retry and
// compaction transitions are observable on the stream when triggered.
#[tokio::test]
async fn runner_transition_events_are_emitted_with_session_and_turn_ids() {
    let scratch = Scratch::new("server-event-transitions");
    let bus = Arc::new(EventBus::new());
    let provider = Arc::new(StreamingEchoProvider);
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
                .body(Body::from(
                    r#"{"text":"hi","model":"anthropic/claude-sonnet-4"}"#,
                ))
                .expect("request"),
        )
        .await
        .expect("message response");
    assert_eq!(response.status(), StatusCode::OK);

    let events = collect_until_kind(
        &mut stream,
        "session.next.step.finished",
        Duration::from_secs(5),
    )
    .await;
    for (kind, label) in [
        ("session.next.model.switched", "model"),
        ("session.next.agent.switched", "agent"),
        ("session.next.step.started", "step start"),
        ("session.next.step.finished", "step finish"),
        ("session.next.context.updated", "context"),
    ] {
        assert!(
            events
                .iter()
                .any(|event| event["kind"] == kind && event["session_id"] == session.id),
            "expected {label} transition event ({kind}) with session id: {events:?}"
        );
    }
    let model = events
        .iter()
        .find(|event| event["kind"] == "session.next.model.switched")
        .expect("model.switched present");
    assert_eq!(
        model["model"], "anthropic/claude-sonnet-4",
        "model.switched reports the per-turn model override"
    );

    // Retry and compaction transitions are observable on the stream when triggered (proven here via
    // the publish path the runner/handler use to surface them).
    let mut retry_stream = subscribe(&app).await;
    post_event(
        &app,
        &Event::session_retried(session.id.clone(), "msg_retry", 1, 3, 250, "rate_limit"),
    )
    .await;
    post_event(
        &app,
        &Event::session_compacted(session.id.clone(), "msg_retry", "tail-42", None),
    )
    .await;
    let triggered = collect_until_kind(
        &mut retry_stream,
        "session.compacted",
        Duration::from_secs(3),
    )
    .await;
    assert!(
        triggered
            .iter()
            .any(|event| event["kind"] == "session.next.retried"
                && event["classification"] == "rate_limit"),
        "retry transition observable: {triggered:?}"
    );
    assert!(
        triggered.iter().any(
            |event| event["kind"] == "session.compacted" && event["tail_start_id"] == "tail-42"
        ),
        "compaction transition observable: {triggered:?}"
    );
}

// VAL-GAP-SSE-004: per-subsystem event families are observable with redacted secrets.
#[tokio::test]
async fn subsystem_event_families_are_observable_and_redacted() {
    let scratch = Scratch::new("server-event-subsystems");
    let bus = Arc::new(EventBus::new());
    let state = server::ServerState::new(scratch.db_path()).with_event_bus(bus);
    let app = server::app_with_state(state);
    let session = create_session(&app).await;
    let mut stream = subscribe(&app).await;

    let secret = format!("sk-{}-{}", "or-v1", "c0ffee00".repeat(8));
    let family_events = vec![
        Event::QuestionAsked(events::QuestionAsked {
            session_id: Some(session.id.clone()),
            id: "q1".to_owned(),
            question: "Proceed?".to_owned(),
            options: vec!["yes".to_owned(), "no".to_owned()],
        }),
        Event::todo_updated(
            session.id.clone(),
            serde_json::json!([{ "id": "t1", "content": "do it", "status": "pending" }]),
        ),
        Event::FileEdited(events::FileEdited {
            session_id: Some(session.id.clone()),
            path: "src/lib.rs".to_owned(),
            edit_kind: Some("edit".to_owned()),
        }),
        Event::LspUpdated(events::LspUpdated {
            server: Some("rust".to_owned()),
            path: Some("src/lib.rs".to_owned()),
            diagnostic_count: Some(2),
        }),
        Event::VcsBranchUpdated(events::VcsBranchUpdated {
            branch: "main".to_owned(),
            dirty: Some(true),
        }),
        Event::PtyCreated(events::PtyEvent {
            id: "pty-1".to_owned(),
            // Carry a secret to prove subsystem payloads are redacted on the stream.
            shell: Some(format!("/bin/sh --token {secret}")),
        }),
        Event::InstallationUpdated(events::InstallationUpdated {
            version: "0.1.0".to_owned(),
        }),
        Event::WorktreeReady(events::WorktreeReady {
            path: "/tmp/worktree".to_owned(),
        }),
        Event::WorkspaceStatus(events::WorkspaceStatus {
            status: "ready".to_owned(),
        }),
        Event::TuiToastShow(events::TuiToastShow {
            message: "saved".to_owned(),
            level: Some("info".to_owned()),
        }),
    ];
    for event in &family_events {
        post_event(&app, event).await;
    }

    let events = collect_until_kind(&mut stream, "tui.toast.show", Duration::from_secs(3)).await;
    for kind in [
        "question.asked",
        "todo.updated",
        "file.edited",
        "lsp.updated",
        "vcs.branch.updated",
        "pty.created",
        "installation.updated",
        "worktree.ready",
        "workspace.status",
        "tui.toast.show",
    ] {
        assert!(
            events.iter().any(|event| event["kind"] == kind),
            "subsystem family {kind} should be observable on SSE: {events:?}"
        );
    }
    let delivered = serde_json::to_string(&events).expect("events JSON");
    assert!(
        !delivered.contains(&secret),
        "subsystem payload secret must be redacted: {delivered}"
    );
    assert!(
        delivered.contains(redaction::PLACEHOLDER),
        "redacted subsystem payload should include the placeholder: {delivered}"
    );
}

// gap-hardening-3: wireable SSE families (`file.edited`, `question.*`) now emit from a REAL turn
// action on the live stream, not only via the `/event` POST passthrough. A successful `write` tool
// result during a turn fires `file.edited` carrying the written path.
#[tokio::test]
async fn live_write_tool_turn_emits_file_edited_from_the_real_action() {
    let scratch = Scratch::new("server-event-file-edited");
    let workspace = scratch.dir.join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace dir");
    let bus = Arc::new(EventBus::new());
    let provider = Arc::new(WriteToolProvider::default());
    let state = server::ServerState::with_provider(scratch.db_path(), provider)
        .with_event_bus(bus)
        .with_cwd(workspace.clone());
    let app = server::app_with_state(state);
    let session = create_session(&app).await;
    // Pin a builtin-family model so tool selection includes write/edit regardless of any ambient
    // global cortex config; without this the GPT-family registry (read + apply_patch only) drops
    // the fake provider's write call.
    pin_builtin_family_model(scratch.db_path(), &session.id);
    let mut stream = subscribe(&app).await;

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/session/{}/message", session.id))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"text":"write a file"}"#))
                .expect("request"),
        )
        .await
        .expect("message response");
    assert_eq!(response.status(), StatusCode::OK);

    let events = collect_until_kind(
        &mut stream,
        "session.next.step.finished",
        Duration::from_secs(5),
    )
    .await;
    let edited = events
        .iter()
        .find(|event| event["kind"] == "file.edited")
        .unwrap_or_else(|| {
            panic!("a real write tool result must emit a live file.edited event: {events:?}")
        });
    assert_eq!(edited["session_id"], session.id);
    assert_eq!(edited["edit_kind"], "write");
    assert!(
        edited["path"]
            .as_str()
            .is_some_and(|path| path.ends_with("notes.txt")),
        "file.edited should carry the written path: {edited:?}"
    );
    // The file was actually written by the real tool, proving this is a real action, not a POST.
    assert_eq!(
        std::fs::read_to_string(workspace.join("notes.txt")).expect("written file"),
        "live-emission"
    );
}

// gap-hardening-4: an `apply_patch` *Update* (editing an existing file) must also emit a live
// `file.edited` carrying the edited path. The path is derived from the parsed envelope hunks
// (pre-apply), NOT by re-applying the patch against the already-mutated file, so the common
// "edit existing file" case is no longer silently dropped.
#[tokio::test]
async fn live_apply_patch_update_turn_emits_file_edited_from_the_real_action() {
    let scratch = Scratch::new("server-event-apply-patch-update");
    let workspace = scratch.dir.join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace dir");
    // The file already exists; an Update hunk edits it in place.
    std::fs::write(workspace.join("notes.txt"), "alpha\n").expect("seed file");
    let bus = Arc::new(EventBus::new());
    let provider = Arc::new(ApplyPatchUpdateProvider::default());
    let state = server::ServerState::with_provider(scratch.db_path(), provider)
        .with_event_bus(bus)
        .with_cwd(workspace.clone());
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
                // A GPT-family model materializes the `apply_patch` tool in place of write/edit.
                .body(Body::from(
                    r#"{"text":"edit the file","model":"openai/gpt-4o"}"#,
                ))
                .expect("request"),
        )
        .await
        .expect("message response");
    assert_eq!(response.status(), StatusCode::OK);

    let events = collect_until_kind(
        &mut stream,
        "session.next.step.finished",
        Duration::from_secs(5),
    )
    .await;
    let edited = events
        .iter()
        .find(|event| event["kind"] == "file.edited")
        .unwrap_or_else(|| {
            panic!(
                "an apply_patch Update tool result must emit a live file.edited event: {events:?}"
            )
        });
    assert_eq!(edited["session_id"], session.id);
    assert_eq!(edited["edit_kind"], "apply_patch");
    assert!(
        edited["path"]
            .as_str()
            .is_some_and(|path| path.ends_with("notes.txt")),
        "file.edited should carry the edited path: {edited:?}"
    );
    // The Update was actually applied on disk, proving this is a real action, not a POST.
    assert_eq!(
        std::fs::read_to_string(workspace.join("notes.txt")).expect("edited file"),
        "beta\n"
    );
}

// gap-hardening-3: a real `question` tool call during a turn emits `question.asked` and, once the
// pending ask is answered, `question.replied` on the live stream.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn live_question_tool_turn_emits_question_family_from_the_real_action() {
    let scratch = Scratch::new("server-event-question-family");
    let bus = Arc::new(EventBus::new());
    let provider = Arc::new(QuestionAskingProvider::default());
    let state = server::ServerState::with_provider(scratch.db_path(), provider).with_event_bus(bus);
    let app = server::app_with_state(state);
    let session = create_session(&app).await;
    let mut stream = subscribe(&app).await;

    let post_app = app.clone();
    let session_id = session.id.clone();
    let turn = tokio::spawn(async move {
        post_app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/session/{session_id}/message"))
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(r#"{"text":"ask a question"}"#))
                    .expect("request"),
            )
            .await
            .expect("question turn response")
    });

    // Wait for the question tool to block on a pending ask, then answer it.
    let mut pending_id = None;
    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline {
        let pending_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/session/{}/permission/pending", session.id))
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("pending response");
        let pending_body: serde_json::Value = json_body(pending_response).await;
        if let Some(id) = pending_body["pending"]
            .as_array()
            .and_then(|pending| pending.first())
            .and_then(|ask| ask["id"].as_str())
        {
            pending_id = Some(id.to_owned());
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let pending_id = pending_id.expect("question tool should block on a pending ask");

    let reply = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/session/{}/permission/reply", session.id))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(format!(
                    r#"{{"id":"{pending_id}","decision":"allow","feedback":"Choose option B"}}"#
                )))
                .expect("request"),
        )
        .await
        .expect("reply response");
    assert_eq!(reply.status(), StatusCode::OK);

    let response = tokio::time::timeout(Duration::from_secs(5), turn)
        .await
        .expect("question turn completed")
        .expect("question turn join");
    assert_eq!(response.status(), StatusCode::OK);

    let events = collect_until_kind(&mut stream, "question.replied", Duration::from_secs(5)).await;
    assert!(
        events.iter().any(|event| event["kind"] == "question.asked"
            && event["session_id"] == session.id
            && event["question"] == "Pick deployment strategy"),
        "a real question tool call should emit a live question.asked event: {events:?}"
    );
    assert!(
        events
            .iter()
            .any(|event| event["kind"] == "question.replied" && event["session_id"] == session.id),
        "answering the question should emit a live question.replied event: {events:?}"
    );
}

// gap-hardening-4: a `question` tool call carrying SEVERAL prompts must surface EVERY prompt on the
// live `question.asked` family, not just the first.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn live_multi_question_tool_turn_emits_a_question_asked_per_prompt() {
    let scratch = Scratch::new("server-event-multi-question");
    let bus = Arc::new(EventBus::new());
    let provider = Arc::new(MultiQuestionAskingProvider::default());
    let state = server::ServerState::with_provider(scratch.db_path(), provider).with_event_bus(bus);
    let app = server::app_with_state(state);
    let session = create_session(&app).await;
    let mut stream = subscribe(&app).await;

    let post_app = app.clone();
    let session_id = session.id.clone();
    let turn = tokio::spawn(async move {
        post_app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/session/{session_id}/message"))
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(r#"{"text":"ask questions"}"#))
                    .expect("request"),
            )
            .await
            .expect("question turn response")
    });

    // The whole multi-question call blocks on a single pending ask; answer it to let the turn finish.
    let mut pending_id = None;
    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline {
        let pending_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/session/{}/permission/pending", session.id))
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("pending response");
        let pending_body: serde_json::Value = json_body(pending_response).await;
        if let Some(id) = pending_body["pending"]
            .as_array()
            .and_then(|pending| pending.first())
            .and_then(|ask| ask["id"].as_str())
        {
            pending_id = Some(id.to_owned());
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let pending_id = pending_id.expect("question tool should block on a pending ask");

    let reply = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/session/{}/permission/reply", session.id))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(format!(
                    r#"{{"id":"{pending_id}","decision":"allow","feedback":"A then us"}}"#
                )))
                .expect("request"),
        )
        .await
        .expect("reply response");
    assert_eq!(reply.status(), StatusCode::OK);

    let response = tokio::time::timeout(Duration::from_secs(5), turn)
        .await
        .expect("question turn completed")
        .expect("question turn join");
    assert_eq!(response.status(), StatusCode::OK);

    let events = collect_until_kind(&mut stream, "question.replied", Duration::from_secs(5)).await;
    let asked: Vec<&str> = events
        .iter()
        .filter(|event| event["kind"] == "question.asked" && event["session_id"] == session.id)
        .filter_map(|event| event["question"].as_str())
        .collect();
    assert!(
        asked.contains(&"Pick deployment strategy") && asked.contains(&"Pick region"),
        "every prompt of a multi-question call should emit a live question.asked event: {events:?}"
    );
}

// gap-hardening-3: message/session mutation routes emit their wireable families from the real
// route action (not only via the `/event` POST): `session.updated` on update, `message.removed`
// on message delete.
#[tokio::test]
async fn live_session_and_message_routes_emit_their_families() {
    let scratch = Scratch::new("server-event-session-message");
    let bus = Arc::new(EventBus::new());
    let provider = Arc::new(StreamingEchoProvider);
    let state = server::ServerState::with_provider(scratch.db_path(), provider).with_event_bus(bus);
    let app = server::app_with_state(state);
    let session = create_session(&app).await;

    // Create a user+assistant message pair through a real turn.
    let turn = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/session/{}/message", session.id))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"text":"hello"}"#))
                .expect("request"),
        )
        .await
        .expect("turn response");
    assert_eq!(turn.status(), StatusCode::OK);

    let mut stream = subscribe(&app).await;

    // Updating the session emits `session.updated`.
    let update = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PATCH")
                .uri(format!("/session/{}", session.id))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"title":"Renamed"}"#))
                .expect("request"),
        )
        .await
        .expect("update response");
    assert_eq!(update.status(), StatusCode::OK);

    // Deleting a message emits `message.removed`.
    let messages_response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/session/{}/message", session.id))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("messages response");
    assert_eq!(messages_response.status(), StatusCode::OK);
    let messages_body: serde_json::Value = json_body(messages_response).await;
    let first_message_id = messages_body["messages"]
        .as_array()
        .and_then(|messages| messages.first())
        .and_then(|message| message["id"].as_str())
        .expect("a persisted message id")
        .to_owned();
    let delete = app
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!(
                    "/session/{}/message/{first_message_id}",
                    session.id
                ))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("delete response");
    assert_eq!(delete.status(), StatusCode::OK);

    let events = collect_until_kind(&mut stream, "message.removed", Duration::from_secs(3)).await;
    assert!(
        events
            .iter()
            .any(|event| event["kind"] == "session.updated" && event["session_id"] == session.id),
        "updating a session should emit a live session.updated event: {events:?}"
    );
    assert!(
        events.iter().any(|event| event["kind"] == "message.removed"
            && event["session_id"] == session.id
            && event["message_id"] == first_message_id),
        "deleting a message should emit a live message.removed event: {events:?}"
    );
}

// gap-hardening-4: deleting a single rich message part emits a live `message.part.removed` event on
// the SSE stream (the route's wireable family, fired from the real DELETE action).
#[tokio::test]
async fn live_delete_part_route_emits_message_part_removed() {
    let scratch = Scratch::new("server-event-part-removed");
    let bus = Arc::new(EventBus::new());
    let provider = Arc::new(StreamingEchoProvider);
    let state = server::ServerState::with_provider(scratch.db_path(), provider).with_event_bus(bus);
    let app = server::app_with_state(state);
    let session = create_session(&app).await;

    // Run a real turn so a persisted message with rich parts exists.
    let turn = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/session/{}/message", session.id))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"text":"hello"}"#))
                .expect("request"),
        )
        .await
        .expect("turn response");
    assert_eq!(turn.status(), StatusCode::OK);

    // Locate a persisted message that carries at least one part.
    let messages_response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/session/{}/message", session.id))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("messages response");
    assert_eq!(messages_response.status(), StatusCode::OK);
    let messages_body: serde_json::Value = json_body(messages_response).await;
    let mut target = None;
    for message in messages_body["messages"]
        .as_array()
        .expect("messages array")
    {
        let message_id = message["id"].as_str().expect("message id");
        if let Some(part_id) = message["parts"]
            .as_array()
            .and_then(|parts| parts.first())
            .and_then(|part| part["id"].as_str())
        {
            target = Some((message_id.to_owned(), part_id.to_owned()));
            break;
        }
    }
    let (message_id, part_id) = target.expect("a persisted message with a part");

    let mut stream = subscribe(&app).await;

    let delete = app
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!(
                    "/session/{}/message/{message_id}/part/{part_id}",
                    session.id
                ))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("delete part response");
    assert_eq!(delete.status(), StatusCode::OK);
    let delete_body: serde_json::Value = json_body(delete).await;
    assert_eq!(delete_body["deleted"], true);

    let events =
        collect_until_kind(&mut stream, "message.part.removed", Duration::from_secs(3)).await;
    assert!(
        events
            .iter()
            .any(|event| event["kind"] == "message.part.removed"
                && event["session_id"] == session.id
                && event["message_id"] == message_id
                && event["part_id"] == part_id),
        "deleting a message part should emit a live message.part.removed event: {events:?}"
    );
}

// VAL-GAP-SSE-005: the SSE event taxonomy is spec-discoverable and every family decodes into a
// distinct typed variant, so a generated SDK client never special-cases server-only names.
#[tokio::test]
async fn event_taxonomy_is_spec_discoverable_and_decodes_into_typed_variants() {
    let scratch = Scratch::new("server-event-taxonomy");
    let app = server::app_with_db_path(scratch.db_path());

    let response = app
        .oneshot(
            Request::builder()
                .uri("/openapi.json")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("openapi response");
    assert_eq!(response.status(), StatusCode::OK);
    let document: serde_json::Value = json_body(response).await;

    let families: Vec<String> = document["x-event-families"]
        .as_array()
        .expect("x-event-families array")
        .iter()
        .map(|value| value.as_str().expect("family string").to_owned())
        .collect();
    let canonical: Vec<String> = events::Event::all_kinds()
        .iter()
        .map(|kind| (*kind).to_owned())
        .collect();
    assert_eq!(
        families, canonical,
        "OpenAPI x-event-families stays in lockstep with the typed Event taxonomy"
    );
    let schema_enum: Vec<String> = document["components"]["schemas"]["Event"]["properties"]["kind"]
        ["enum"]
        .as_array()
        .expect("Event schema kind enum")
        .iter()
        .map(|value| value.as_str().expect("enum string").to_owned())
        .collect();
    assert_eq!(
        schema_enum, canonical,
        "the Event schema enum documents every typed family"
    );

    // Each representative family round-trips: serialize → deserialize into the typed Event enum and
    // confirm the discriminator resolves to the same variant the SDK would dispatch on.
    for event in representative_events() {
        let wire = serde_json::to_string(&event).expect("serialize event");
        let decoded: Event = serde_json::from_str(&wire).expect("typed client decodes family");
        assert_eq!(
            decoded.kind(),
            event.kind(),
            "family {} decodes into its typed variant",
            event.kind()
        );
    }
    // The representatives cover every advertised family.
    let covered: std::collections::BTreeSet<&str> =
        representative_events().iter().map(Event::kind).collect();
    for kind in events::Event::all_kinds() {
        assert!(
            covered.contains(kind),
            "representative coverage missing family {kind}"
        );
    }
}

/// One representative [`Event`] per family, used to prove typed decodability of the full taxonomy.
fn representative_events() -> Vec<Event> {
    use events::{
        FileWatcherUpdated, MessagePartRemoved, MessageRemoved, ProjectUpdated, PtyExited, PtyRef,
        QuestionAsked, QuestionRejected, QuestionReplied, SessionRef, ToolEnded, ToolStarted,
        TuiCommandExecute, TuiPromptAppend, TuiSessionSelect, WorkspaceEvent, WorktreeFailed,
    };
    let part = protocol::MessagePart::text("hi");
    vec![
        Event::lifecycle(
            events::LifecyclePhase::Startup,
            events::Surface::Serve,
            "0.1.0",
        ),
        Event::permission_asked("bash", "rm *", "rm a"),
        Event::permission_replied(
            "bash",
            "rm *",
            "rm a",
            events::PermissionDecision::Deny,
            None,
        ),
        Event::mcp_tools_changed("demo", vec!["a".to_owned()]),
        Event::mcp_resources_changed("demo", vec!["notes://demo".to_owned()]),
        Event::mcp_prompts_changed("demo", vec!["greet".to_owned()]),
        Event::mcp_log_message("demo", "info", None, serde_json::json!({"m":"x"})),
        Event::tool_call("c1", "bash", "{}"),
        Event::tool_result("c1", "bash", false),
        Event::turn_started("s", "m"),
        Event::assistant_text_delta("s", "m", "hi"),
        Event::assistant_reasoning_delta("s", "m", "think"),
        Event::turn_usage("s", "m", 1, 1, 2, None, None, None),
        Event::turn_error("s", "m", "upstream", "boom"),
        Event::turn_done("s", "m", "stop", 1),
        Event::message_parts("s", "m", vec![part.clone()]),
        Event::message_updated("s", "m", "assistant", None),
        Event::MessageRemoved(MessageRemoved {
            session_id: "s".to_owned(),
            message_id: "m".to_owned(),
        }),
        Event::message_part_updated("s", "m", part.clone()),
        Event::MessagePartRemoved(MessagePartRemoved {
            session_id: "s".to_owned(),
            message_id: "m".to_owned(),
            part_id: "p".to_owned(),
        }),
        Event::SessionNextTextStart(boundary("s", "m")),
        Event::SessionNextTextDelta(delta("s", "m", "hi")),
        Event::SessionNextTextEnd(boundary("s", "m")),
        Event::SessionNextReasoningStart(boundary("s", "m")),
        Event::SessionNextReasoningDelta(delta("s", "m", "think")),
        Event::SessionNextReasoningEnd(boundary("s", "m")),
        Event::SessionNextToolStarted(ToolStarted {
            session_id: "s".to_owned(),
            message_id: "m".to_owned(),
            id: "c1".to_owned(),
            name: "bash".to_owned(),
        }),
        Event::SessionNextToolEnded(ToolEnded {
            session_id: "s".to_owned(),
            message_id: "m".to_owned(),
            id: "c1".to_owned(),
            name: "bash".to_owned(),
            is_error: false,
        }),
        Event::session_step_started("s", "m", 1),
        Event::session_step_finished("s", "m", 1),
        Event::session_retried("s", "m", 1, 3, 250, "rate_limit"),
        Event::session_context_updated("s", "m", 10, Some(1000)),
        Event::session_model_switched("s", "m", "google/gemini-2.5-flash"),
        Event::session_agent_switched("s", "m", "build"),
        Event::SessionUpdated(SessionRef {
            session_id: "s".to_owned(),
        }),
        Event::session_status("s", "busy"),
        Event::session_idle("s"),
        Event::session_error("s", Some("m".to_owned()), "upstream", "boom"),
        Event::SessionDiff(events::SessionDiffEvent {
            session_id: "s".to_owned(),
            files: vec!["a.rs".to_owned()],
        }),
        Event::session_compacted("s", "m", "tail-1", None),
        Event::QuestionAsked(QuestionAsked {
            session_id: None,
            id: "q".to_owned(),
            question: "?".to_owned(),
            options: Vec::new(),
        }),
        Event::QuestionReplied(QuestionReplied {
            session_id: None,
            id: "q".to_owned(),
            answer: "y".to_owned(),
        }),
        Event::QuestionRejected(QuestionRejected {
            session_id: None,
            id: "q".to_owned(),
            reason: None,
        }),
        Event::todo_updated("s", serde_json::json!([])),
        Event::file_edited(None, "a.rs", Some("edit".to_owned())),
        Event::FileWatcherUpdated(FileWatcherUpdated {
            path: "a.rs".to_owned(),
            change: "modified".to_owned(),
        }),
        Event::LspUpdated(events::LspUpdated {
            server: Some("rust".to_owned()),
            path: None,
            diagnostic_count: None,
        }),
        Event::VcsBranchUpdated(events::VcsBranchUpdated {
            branch: "main".to_owned(),
            dirty: None,
        }),
        Event::ProjectUpdated(ProjectUpdated {
            path: "/p".to_owned(),
        }),
        Event::command_executed(None, "echo hi", Some(0)),
        Event::InstallationUpdated(events::InstallationUpdated {
            version: "0.1.0".to_owned(),
        }),
        Event::WorktreeReady(events::WorktreeReady {
            path: "/w".to_owned(),
        }),
        Event::WorktreeFailed(WorktreeFailed {
            path: "/w".to_owned(),
            message: "bad".to_owned(),
        }),
        Event::worktree_removed("/w"),
        Event::WorkspaceReady(WorkspaceEvent { id: None }),
        Event::WorkspaceFailed(WorkspaceEvent { id: None }),
        Event::WorkspaceStatus(events::WorkspaceStatus {
            status: "ready".to_owned(),
        }),
        Event::PtyCreated(events::PtyEvent {
            id: "p".to_owned(),
            shell: None,
        }),
        Event::PtyUpdated(events::PtyEvent {
            id: "p".to_owned(),
            shell: None,
        }),
        Event::PtyDeleted(PtyRef { id: "p".to_owned() }),
        Event::PtyExited(PtyExited {
            id: "p".to_owned(),
            exit_code: Some(0),
        }),
        Event::TuiPromptAppend(TuiPromptAppend {
            text: "hi".to_owned(),
        }),
        Event::TuiCommandExecute(TuiCommandExecute {
            command: "help".to_owned(),
        }),
        Event::TuiToastShow(events::TuiToastShow {
            message: "saved".to_owned(),
            level: None,
        }),
        Event::TuiSessionSelect(TuiSessionSelect {
            session_id: "s".to_owned(),
        }),
    ]
}

fn boundary(session_id: &str, message_id: &str) -> events::StreamBoundary {
    events::StreamBoundary {
        session_id: session_id.to_owned(),
        message_id: message_id.to_owned(),
    }
}

fn delta(session_id: &str, message_id: &str, text: &str) -> events::StreamDelta {
    events::StreamDelta {
        session_id: session_id.to_owned(),
        message_id: message_id.to_owned(),
        text: text.to_owned(),
    }
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

async fn post_event(app: &axum::Router, event: &Event) {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/event")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    serde_json::to_string(event).expect("event JSON"),
                ))
                .expect("request"),
        )
        .await
        .expect("event post response");
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value = json_body(response).await;
    assert_eq!(body["status"], "published");
    assert_eq!(body["kind"], event.kind());
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

/// Collect SSE data events until one with `kind` appears (inclusive) or the timeout elapses.
async fn collect_until_kind(
    stream: &mut axum::body::BodyDataStream,
    kind: &str,
    timeout: Duration,
) -> Vec<serde_json::Value> {
    let start = Instant::now();
    let mut buffer = String::new();
    let mut events: Vec<serde_json::Value> = Vec::new();
    while start.elapsed() < timeout {
        if events.iter().any(|event| event["kind"] == kind) {
            break;
        }
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

async fn json_body<T>(response: axum::response::Response) -> T
where
    T: serde::de::DeserializeOwned,
{
    let bytes = to_bytes(response.into_body(), 4_194_304)
        .await
        .expect("body");
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
                    cache_creation_input_tokens: None,
                    cache_read_input_tokens: None,
                    reasoning_output_tokens: None,
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
                        cache_creation_input_tokens: None,
                        cache_read_input_tokens: None,
                        reasoning_output_tokens: None,
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

/// Emits a single `write` tool call, then a final answer once the tool result is observed.
#[derive(Debug, Default)]
struct WriteToolProvider {
    calls: AtomicUsize,
}

impl Provider for WriteToolProvider {
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
                        id: "call_write".to_owned(),
                        name: "write".to_owned(),
                        arguments: r#"{"path":"notes.txt","content":"live-emission"}"#.to_owned(),
                    }),
                    LlmEvent::Done {
                        reason: FinishReason::ToolCalls,
                    },
                ]
            } else {
                vec![
                    LlmEvent::TextDelta {
                        text: "wrote the file".to_owned(),
                    },
                    LlmEvent::Usage(Usage {
                        input_tokens: 4,
                        output_tokens: 4,
                        total_tokens: 8,
                        cache_creation_input_tokens: None,
                        cache_read_input_tokens: None,
                        reasoning_output_tokens: None,
                        cost: Some(0.000_001),
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

/// Emits a single `question` tool call, then echoes the supplied answer once the tool result lands.
#[derive(Debug, Default)]
struct QuestionAskingProvider {
    calls: AtomicUsize,
}

impl Provider for QuestionAskingProvider {
    fn stream<'a>(
        &'a self,
        request: LlmRequest,
    ) -> BoxFuture<'a, Result<LlmStream, llm::LlmError>> {
        Box::pin(async move {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let has_tool_result = request.messages.iter().any(|message| {
                message.role == Role::Tool
                    && message.tool_call_id.as_deref() == Some("call_question")
            });
            let events = if !has_tool_result {
                vec![
                    LlmEvent::ToolCall(ToolCall {
                        id: "call_question".to_owned(),
                        name: "question".to_owned(),
                        arguments: serde_json::json!({
                            "questions": [{
                                "prompt": "Pick deployment strategy",
                                "options": ["A", "B"]
                            }]
                        })
                        .to_string(),
                    }),
                    LlmEvent::Done {
                        reason: FinishReason::ToolCalls,
                    },
                ]
            } else {
                vec![
                    LlmEvent::TextDelta {
                        text: "question answered".to_owned(),
                    },
                    LlmEvent::Usage(Usage {
                        input_tokens: 6,
                        output_tokens: 4,
                        total_tokens: 10,
                        cache_creation_input_tokens: None,
                        cache_read_input_tokens: None,
                        reasoning_output_tokens: None,
                        cost: Some(0.000_001),
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

/// Emits a `read` then an `apply_patch` tool call that UPDATES an existing file (the common "edit
/// an existing file" case), then a final answer once the tool results land. The read satisfies the
/// read-before-update guard. Exercises the GPT-family `apply_patch` path (no `write`/`edit`).
#[derive(Debug, Default)]
struct ApplyPatchUpdateProvider {
    calls: AtomicUsize,
}

impl Provider for ApplyPatchUpdateProvider {
    fn stream<'a>(
        &'a self,
        request: LlmRequest,
    ) -> BoxFuture<'a, Result<LlmStream, llm::LlmError>> {
        Box::pin(async move {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let saw_read = request.messages.iter().any(|message| {
                message.role == Role::Tool && message.tool_call_id.as_deref() == Some("call_read")
            });
            let saw_patch = request.messages.iter().any(|message| {
                message.role == Role::Tool
                    && message.tool_call_id.as_deref() == Some("call_apply_patch")
            });
            let events = if !saw_read {
                vec![
                    LlmEvent::ToolCall(ToolCall {
                        id: "call_read".to_owned(),
                        name: "read".to_owned(),
                        arguments: serde_json::json!({ "path": "notes.txt" }).to_string(),
                    }),
                    LlmEvent::Done {
                        reason: FinishReason::ToolCalls,
                    },
                ]
            } else if !saw_patch {
                let patch = "*** Begin Patch\n*** Update File: notes.txt\n@@\n-alpha\n+beta\n*** End Patch\n";
                vec![
                    LlmEvent::ToolCall(ToolCall {
                        id: "call_apply_patch".to_owned(),
                        name: "apply_patch".to_owned(),
                        arguments: serde_json::json!({ "patch": patch }).to_string(),
                    }),
                    LlmEvent::Done {
                        reason: FinishReason::ToolCalls,
                    },
                ]
            } else {
                vec![
                    LlmEvent::TextDelta {
                        text: "patched the file".to_owned(),
                    },
                    LlmEvent::Usage(Usage {
                        input_tokens: 4,
                        output_tokens: 4,
                        total_tokens: 8,
                        cache_creation_input_tokens: None,
                        cache_read_input_tokens: None,
                        reasoning_output_tokens: None,
                        cost: Some(0.000_001),
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

/// Emits a single `question` tool call carrying MULTIPLE prompts, then echoes the answer once the
/// tool result lands.
#[derive(Debug, Default)]
struct MultiQuestionAskingProvider {
    calls: AtomicUsize,
}

impl Provider for MultiQuestionAskingProvider {
    fn stream<'a>(
        &'a self,
        request: LlmRequest,
    ) -> BoxFuture<'a, Result<LlmStream, llm::LlmError>> {
        Box::pin(async move {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let has_tool_result = request.messages.iter().any(|message| {
                message.role == Role::Tool
                    && message.tool_call_id.as_deref() == Some("call_question")
            });
            let events = if !has_tool_result {
                vec![
                    LlmEvent::ToolCall(ToolCall {
                        id: "call_question".to_owned(),
                        name: "question".to_owned(),
                        arguments: serde_json::json!({
                            "questions": [
                                {
                                    "prompt": "Pick deployment strategy",
                                    "options": ["A", "B"]
                                },
                                {
                                    "prompt": "Pick region",
                                    "options": ["us", "eu"]
                                }
                            ]
                        })
                        .to_string(),
                    }),
                    LlmEvent::Done {
                        reason: FinishReason::ToolCalls,
                    },
                ]
            } else {
                vec![
                    LlmEvent::TextDelta {
                        text: "questions answered".to_owned(),
                    },
                    LlmEvent::Usage(Usage {
                        input_tokens: 6,
                        output_tokens: 4,
                        total_tokens: 10,
                        cache_creation_input_tokens: None,
                        cache_read_input_tokens: None,
                        reasoning_output_tokens: None,
                        cost: Some(0.000_001),
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

fn pin_builtin_family_model(db_path: PathBuf, session_id: &str) {
    db::Database::open(db_path)
        .expect("open db to pin session model")
        .sessions()
        .set_model(session_id, Some("google/gemini-2.5-flash"))
        .expect("pin builtin-family session model");
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
