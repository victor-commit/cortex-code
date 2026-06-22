use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::Duration;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode, header};
use db::Session;
use futures_util::future::BoxFuture;
use llm::{FinishReason, LlmError, LlmEvent, LlmRequest, LlmStream, Provider, Role, Usage};
use tower::ServiceExt;

static SCRATCH_COUNTER: AtomicU64 = AtomicU64::new(0);

#[tokio::test]
async fn message_route_runs_turn_persists_assistant_and_usage() {
    let scratch = Scratch::new("server-message-turn");
    let provider = Arc::new(EchoProvider::default());
    let app = server::app_with_provider(scratch.db_path(), provider.clone());
    let session = create_session(&app).await;
    let nonce = "nonce-server-message-123";

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/session/{}/message", session.id))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(format!(
                    r#"{{"parts":[{{"type":"text","text":"please echo {nonce}"}}]}}"#
                )))
                .expect("request"),
        )
        .await
        .expect("message response");

    assert_eq!(response.status(), StatusCode::OK);
    assert_json_content_type(&response);
    let body: serde_json::Value = json_body(response).await;
    assert_eq!(body["status"], "completed");
    assert_eq!(body["role"], "assistant");
    assert!(
        body["content"]
            .as_str()
            .is_some_and(|text| text.contains(nonce)),
        "assistant content should contain nonce: {body:?}"
    );
    assert_eq!(body["message"]["role"], "assistant");
    assert_eq!(body["usage"]["input_tokens"], 11);
    assert_eq!(body["usage"]["output_tokens"], 7);
    assert_eq!(provider.calls(), 1);

    let response = app
        .oneshot(
            Request::builder()
                .uri(format!("/session/{}/message", session.id))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("message list response");
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value = json_body(response).await;
    let messages = body["messages"].as_array().expect("messages array");
    assert_eq!(messages.len(), 2, "user + assistant should persist");
    assert_eq!(messages[0]["role"], "user");
    assert_eq!(messages[1]["role"], "assistant");
    assert!(
        messages[1]["content"]
            .as_str()
            .is_some_and(|text| text.contains(nonce)),
        "persisted assistant should contain nonce: {body:?}"
    );
    assert_eq!(body["usage"]["output_tokens"], 7);
}

#[tokio::test]
async fn prompt_append_updates_draft_without_starting_turn() {
    let scratch = Scratch::new("server-prompt-append");
    let provider = Arc::new(EchoProvider::default());
    let app = server::app_with_provider(scratch.db_path(), provider.clone());
    let session = create_session(&app).await;

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/session/{}/prompt/append", session.id))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"text":"hello-from-remote"}"#))
                .expect("request"),
        )
        .await
        .expect("append response");

    assert_eq!(response.status(), StatusCode::OK);
    assert_json_content_type(&response);
    let body: serde_json::Value = json_body(response).await;
    assert_eq!(body["status"], "updated");
    assert_eq!(body["text"], "hello-from-remote");
    assert_eq!(provider.calls(), 0, "append must not submit a turn");

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/session/{}/prompt", session.id))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("draft response");
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value = json_body(response).await;
    assert_eq!(body["text"], "hello-from-remote");

    let response = app
        .oneshot(
            Request::builder()
                .uri(format!("/session/{}/message", session.id))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("message list response");
    let body: serde_json::Value = json_body(response).await;
    assert!(
        body["messages"].as_array().expect("messages").is_empty(),
        "draft append must not persist user or assistant messages: {body:?}"
    );
}

#[tokio::test]
async fn prompt_submit_runs_current_draft_and_clears_buffer() {
    let scratch = Scratch::new("server-prompt-submit");
    let provider = Arc::new(EchoProvider::default());
    let app = server::app_with_provider(scratch.db_path(), provider.clone());
    let session = create_session(&app).await;

    let append = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/session/{}/prompt/append", session.id))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"text":"Reply with exactly: REMOTE-OK"}"#))
                .expect("request"),
        )
        .await
        .expect("append response");
    assert_eq!(append.status(), StatusCode::OK);

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/session/{}/prompt/submit", session.id))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("submit response");

    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value = json_body(response).await;
    assert_eq!(body["status"], "completed");
    assert!(
        body["content"]
            .as_str()
            .is_some_and(|content| content.contains("REMOTE-OK")),
        "assistant should answer from the appended draft: {body:?}"
    );
    assert_eq!(provider.calls(), 1);

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/session/{}/prompt", session.id))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("draft response");
    let body: serde_json::Value = json_body(response).await;
    assert_eq!(body["text"], "", "submit should clear the pending draft");

    let response = app
        .oneshot(
            Request::builder()
                .uri(format!("/session/{}/message", session.id))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("message list response");
    let body: serde_json::Value = json_body(response).await;
    let messages = body["messages"].as_array().expect("messages");
    assert_eq!(messages.len(), 2, "submit should persist user + assistant");
    assert_eq!(messages[0]["role"], "user");
    assert!(
        messages[0]["content"]
            .as_str()
            .is_some_and(|content| content.contains("REMOTE-OK")),
        "user message should be the submitted draft: {body:?}"
    );
    assert_eq!(messages[1]["role"], "assistant");
}

#[tokio::test]
async fn prompt_clear_discards_pending_draft_without_turn() {
    let scratch = Scratch::new("server-prompt-clear");
    let provider = Arc::new(EchoProvider::default());
    let app = server::app_with_provider(scratch.db_path(), provider.clone());
    let session = create_session(&app).await;

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/session/{}/prompt/append", session.id))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"text":"discard-me"}"#))
                .expect("request"),
        )
        .await
        .expect("append response");
    assert_eq!(response.status(), StatusCode::OK);

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/session/{}/prompt/clear", session.id))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("clear response");
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value = json_body(response).await;
    assert_eq!(body["status"], "cleared");
    assert_eq!(body["text"], "");
    assert_eq!(provider.calls(), 0, "clear must not submit a turn");

    let response = app
        .oneshot(
            Request::builder()
                .uri(format!("/session/{}/prompt", session.id))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("draft response");
    let body: serde_json::Value = json_body(response).await;
    assert_eq!(body["text"], "", "cleared draft must read back empty");
}

#[tokio::test]
async fn command_route_runs_shell_command_and_returns_output() {
    let scratch = Scratch::new("server-command");
    let app = server::app_with_db_path(scratch.db_path());
    let session = create_session(&app).await;

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/session/{}/command", session.id))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"command":"echo cortex-exec-marker"}"#))
                .expect("request"),
        )
        .await
        .expect("command response");

    assert_eq!(response.status(), StatusCode::OK);
    assert_json_content_type(&response);
    let body: serde_json::Value = json_body(response).await;
    assert_eq!(body["status"], "completed");
    assert_eq!(body["is_error"], false);
    assert!(
        body["output"]
            .as_str()
            .is_some_and(|output| output.contains("cortex-exec-marker")),
        "command output should contain the marker: {body:?}"
    );
}

#[tokio::test]
async fn share_route_serves_conversation_over_loopback_and_unshare_revokes() {
    let scratch = Scratch::new("server-share");
    let app = server::app_with_provider(scratch.db_path(), Arc::new(EchoProvider::default()));
    let session = create_session(&app).await;
    let nonce = "nonce-local-share-123";

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/session/{}/message", session.id))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(format!(r#"{{"text":"please echo {nonce}"}}"#)))
                .expect("request"),
        )
        .await
        .expect("message response");
    assert_eq!(response.status(), StatusCode::OK);

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/session/{}/share", session.id))
                .header(header::HOST, "127.0.0.1:3201")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("share response");

    assert_eq!(response.status(), StatusCode::OK);
    assert_json_content_type(&response);
    let body: serde_json::Value = json_body(response).await;
    assert_eq!(body["status"], "shared");
    assert!(
        body["share_id"].as_str().is_some_and(|id| !id.is_empty()),
        "share id should be non-empty: {body:?}"
    );
    let url = body["url"].as_str().expect("share url");
    assert!(
        url.starts_with("http://127.0.0.1:3201/share/"),
        "share URL must point at the loopback server, got {url:?}"
    );

    let share_path = url
        .strip_prefix("http://127.0.0.1:3201")
        .expect("loopback URL prefix");
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(share_path)
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("shared page response");
    assert_eq!(response.status(), StatusCode::OK);
    let html = text_body(response).await;
    assert!(
        html.contains(nonce),
        "shared page should render assistant content containing nonce: {html}"
    );
    assert!(
        html.contains("assistant"),
        "shared page should label assistant messages: {html}"
    );

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/session/{}/unshare", session.id))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("unshare response");
    assert_eq!(response.status(), StatusCode::OK);
    assert_json_content_type(&response);
    let body: serde_json::Value = json_body(response).await;
    assert_eq!(body["status"], "unshared");
    assert_eq!(body["revoked"], true);

    let response = app
        .oneshot(
            Request::builder()
                .uri(share_path)
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("revoked share response");
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let body = text_body(response).await;
    assert!(
        !body.contains(nonce),
        "revoked share must not keep serving the session content: {body}"
    );
}

#[tokio::test]
async fn share_rejects_non_loopback_host_reference() {
    let scratch = Scratch::new("server-share-host");
    let app = server::app_with_db_path(scratch.db_path());
    let session = create_session(&app).await;

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/session/{}/share", session.id))
                .header(header::HOST, "share.example.com:443")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("share response");

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_json_content_type(&response);
    let body: serde_json::Value = json_body(response).await;
    assert!(
        body["error"]
            .as_str()
            .is_some_and(|message| message.contains("loopback")),
        "non-loopback host should be rejected clearly: {body:?}"
    );
}

#[tokio::test]
async fn prompt_to_unknown_session_returns_404_without_starting_turn() {
    let scratch = Scratch::new("server-message-unknown");
    let provider = Arc::new(EchoProvider::default());
    let app = server::app_with_provider(scratch.db_path(), provider.clone());
    let unknown_id = format!("missing-{}", std::process::id());

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/session/{unknown_id}/message"))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"text":"hi"}"#))
                .expect("request"),
        )
        .await
        .expect("unknown message response");

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert_json_content_type(&response);
    let body: serde_json::Value = json_body(response).await;
    assert!(
        body["error"]
            .as_str()
            .is_some_and(|message| message.contains(&unknown_id)),
        "404 body should name unknown session: {body:?}"
    );
    assert_eq!(provider.calls(), 0, "unknown session must not start a turn");
}

#[tokio::test]
async fn abort_stops_in_flight_turn_without_assistant_completion() {
    let scratch = Scratch::new("server-message-abort");
    let provider = Arc::new(EchoProvider::with_delay(Duration::from_secs(30)));
    let app = server::app_with_provider(scratch.db_path(), provider);
    let session = create_session(&app).await;

    let post_app = app.clone();
    let post_uri = format!("/session/{}/message", session.id);
    let post = tokio::spawn(async move {
        post_app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(post_uri)
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(r#"{"text":"Count slowly from 1 to 300."}"#))
                    .expect("request"),
            )
            .await
            .expect("post response")
    });

    tokio::time::sleep(Duration::from_millis(50)).await;
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
    assert!(response.status().is_success());
    let abort_body: serde_json::Value = json_body(response).await;
    assert_eq!(abort_body["status"], "abort_requested");
    assert_eq!(abort_body["active"], true);

    let response = tokio::time::timeout(Duration::from_secs(3), post)
        .await
        .expect("prompt stopped promptly")
        .expect("prompt task joined");
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value = json_body(response).await;
    assert_eq!(body["status"], "aborted");
    assert!(
        body.get("content").is_none(),
        "aborted turn must not fabricate assistant content: {body:?}"
    );

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/health")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("health response");
    assert_eq!(response.status(), StatusCode::OK);

    let response = app
        .oneshot(
            Request::builder()
                .uri(format!("/session/{}/message", session.id))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("message list response");
    let body: serde_json::Value = json_body(response).await;
    let messages = body["messages"].as_array().expect("messages array");
    assert!(
        messages
            .iter()
            .all(|message| message["role"] != "assistant"),
        "aborted turn should not persist a completed assistant: {body:?}"
    );
}

#[tokio::test]
async fn concurrent_prompts_to_distinct_sessions_are_isolated() {
    let scratch = Scratch::new("server-message-concurrent");
    let app = server::app_with_provider(scratch.db_path(), Arc::new(EchoProvider::default()));
    let sessions = [
        create_session(&app).await,
        create_session(&app).await,
        create_session(&app).await,
    ];
    let nonces = ["nonce-alpha", "nonce-bravo", "nonce-charlie"];

    let mut tasks = Vec::new();
    for (session, nonce) in sessions.iter().zip(nonces) {
        let app = app.clone();
        let uri = format!("/session/{}/message", session.id);
        tasks.push(tokio::spawn(async move {
            let response = app
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri(uri)
                        .header(header::CONTENT_TYPE, "application/json")
                        .body(Body::from(format!(r#"{{"text":"return {nonce}"}}"#)))
                        .expect("request"),
                )
                .await
                .expect("message response");
            (nonce, response)
        }));
    }

    for task in tasks {
        let (nonce, response) = task.await.expect("task joined");
        assert_eq!(response.status(), StatusCode::OK);
        let body: serde_json::Value = json_body(response).await;
        let content = body["content"].as_str().expect("assistant content");
        assert!(
            content.contains(nonce),
            "response for {nonce} should contain its own nonce: {body:?}"
        );
        for other in nonces {
            if other != nonce {
                assert!(
                    !content.contains(other),
                    "response for {nonce} must not contain {other}: {body:?}"
                );
            }
        }
    }

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
}

#[tokio::test]
async fn messages_survive_app_recreation_on_same_database() {
    let scratch = Scratch::new("server-message-durable");
    let provider = Arc::new(EchoProvider::default());
    let first_app = server::app_with_provider(scratch.db_path(), provider.clone());
    let session = create_session(&first_app).await;
    let nonce = "nonce-durable-restart";

    let response = first_app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/session/{}/message", session.id))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(format!(r#"{{"text":"echo {nonce}"}}"#)))
                .expect("request"),
        )
        .await
        .expect("message response");
    assert_eq!(response.status(), StatusCode::OK);

    let relaunched_app = server::app_with_provider(scratch.db_path(), provider);
    let response = relaunched_app
        .oneshot(
            Request::builder()
                .uri(format!("/session/{}/message", session.id))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("message list response");
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value = json_body(response).await;
    assert!(
        body["messages"]
            .as_array()
            .expect("messages")
            .iter()
            .any(|message| message["role"] == "assistant"
                && message["content"]
                    .as_str()
                    .is_some_and(|content| content.contains(nonce))),
        "recreated app should serve prior assistant content: {body:?}"
    );
}

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

async fn text_body(response: axum::response::Response) -> String {
    let bytes = to_bytes(response.into_body(), 16_384).await.expect("body");
    String::from_utf8(bytes.to_vec()).expect("UTF-8 body")
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
struct EchoProvider {
    calls: AtomicUsize,
    delay: Option<Duration>,
}

impl EchoProvider {
    fn with_delay(delay: Duration) -> Self {
        Self {
            calls: AtomicUsize::new(0),
            delay: Some(delay),
        }
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl Provider for EchoProvider {
    fn stream<'a>(&'a self, request: LlmRequest) -> BoxFuture<'a, Result<LlmStream, LlmError>> {
        Box::pin(async move {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if let Some(delay) = self.delay {
                tokio::time::sleep(delay).await;
            }
            let prompt = request
                .messages
                .iter()
                .rev()
                .find(|message| message.role == Role::User)
                .map(|message| message.content.clone())
                .unwrap_or_default();
            let answer = format!("assistant saw: {prompt}");
            let events = vec![
                LlmEvent::TextDelta { text: answer },
                LlmEvent::Usage(Usage {
                    input_tokens: 11,
                    output_tokens: 7,
                    total_tokens: 18,
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
