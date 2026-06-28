//! Black-box integration coverage for the extended session route family
//! (VAL-GAP-SERVER-001..006): update, cursor paging, message/part get/delete/update, status/wait/
//! context, prompt_async, and fork/children/diff/summarize/compact/revert/unrevert.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode, header};
use db::Session;
use futures_util::future::BoxFuture;
use llm::{
    FinishReason, LlmError, LlmEvent, LlmRequest, LlmStream, Provider, Role, ToolCall, Usage,
};
use tower::ServiceExt;

static SCRATCH_COUNTER: AtomicU64 = AtomicU64::new(0);

#[tokio::test]
async fn update_session_route_modifies_title_and_model_and_persists() {
    let scratch = Scratch::new("server-update-session");
    let app = server::app_with_db_path(scratch.db_path());
    let session = create_session(&app).await;

    let response = send(
        &app,
        "PATCH",
        &format!("/session/{}", session.id),
        Some(r#"{"title":"renamed session","model":"google/gemini-2.5-flash"}"#),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value = json_body(response).await;
    assert_eq!(body["title"], "renamed session");
    assert_eq!(body["model"], "google/gemini-2.5-flash");

    // Later GET reflects the new values.
    let response = send(&app, "GET", &format!("/session/{}", session.id), None).await;
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value = json_body(response).await;
    assert_eq!(body["title"], "renamed session");
    assert_eq!(body["model"], "google/gemini-2.5-flash");

    // An empty patch is a clean client error.
    let response = send(
        &app,
        "PATCH",
        &format!("/session/{}", session.id),
        Some("{}"),
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    // A malformed model is rejected.
    let response = send(
        &app,
        "PATCH",
        &format!("/session/{}", session.id),
        Some(r#"{"model":"not a valid model id"}"#),
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    // Values persist across a fresh app over the same database.
    let relaunched = server::app_with_db_path(scratch.db_path());
    let response = send(
        &relaunched,
        "GET",
        &format!("/session/{}", session.id),
        None,
    )
    .await;
    let body: serde_json::Value = json_body(response).await;
    assert_eq!(body["title"], "renamed session");
    assert_eq!(body["model"], "google/gemini-2.5-flash");
}

#[tokio::test]
async fn message_list_supports_cursor_and_limit_in_stable_order() {
    let scratch = Scratch::new("server-message-paging");
    let app = server::app_with_provider(scratch.db_path(), Arc::new(EchoProvider::default()));
    let session = create_session(&app).await;
    for index in 0..3 {
        let response = send(
            &app,
            "POST",
            &format!("/session/{}/message", session.id),
            Some(&format!(r#"{{"text":"turn {index}"}}"#)),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
    }

    // Full transcript order is the baseline.
    let response = send(
        &app,
        "GET",
        &format!("/session/{}/message", session.id),
        None,
    )
    .await;
    let body: serde_json::Value = json_body_large(response).await;
    let full: Vec<String> = body["messages"]
        .as_array()
        .expect("messages")
        .iter()
        .map(|message| message["id"].as_str().expect("id").to_owned())
        .collect();
    assert_eq!(
        full.len(),
        6,
        "three echo turns persist user+assistant each"
    );

    // Page forward by cursor with a fixed limit, collecting ids exactly once in order.
    let mut paged: Vec<String> = Vec::new();
    let mut cursor: Option<String> = None;
    loop {
        let uri = match &cursor {
            Some(cursor) => format!("/session/{}/message?limit=2&cursor={cursor}", session.id),
            None => format!("/session/{}/message?limit=2", session.id),
        };
        let response = send(&app, "GET", &uri, None).await;
        assert_eq!(response.status(), StatusCode::OK);
        let body: serde_json::Value = json_body_large(response).await;
        let page: Vec<String> = body["messages"]
            .as_array()
            .expect("messages")
            .iter()
            .map(|message| message["id"].as_str().expect("id").to_owned())
            .collect();
        assert!(page.len() <= 2, "limit caps page size: {page:?}");
        paged.extend(page);
        match body["next_cursor"].as_str() {
            Some(next) => cursor = Some(next.to_owned()),
            None => break,
        }
    }
    assert_eq!(
        paged, full,
        "paging covers every message once in transcript order"
    );

    // An unknown cursor is a clean client error.
    let response = send(
        &app,
        "GET",
        &format!("/session/{}/message?cursor=does-not-exist", session.id),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn message_get_delete_and_part_update_delete_are_targeted() {
    let scratch = Scratch::new("server-message-parts");
    let app = server::app_with_provider(scratch.db_path(), Arc::new(RichPartsProvider::default()));
    let session = create_session(&app).await;
    let response = send(
        &app,
        "POST",
        &format!("/session/{}/message", session.id),
        Some(r#"{"text":"emit rich parts"}"#),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value = json_body_large(response).await;
    let message_id = body["message"]["id"]
        .as_str()
        .expect("message id")
        .to_owned();

    // GET returns the exact message with its parts.
    let response = send(
        &app,
        "GET",
        &format!("/session/{}/message/{}", session.id, message_id),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let message: serde_json::Value = json_body_large(response).await;
    assert_eq!(message["id"], message_id);
    let parts = message["parts"].as_array().expect("parts").clone();
    let text_part = parts
        .iter()
        .find(|part| part["type"] == "text")
        .expect("a text part");
    let text_part_id = text_part["id"].as_str().expect("text part id").to_owned();
    let other_ids: Vec<String> = parts
        .iter()
        .filter(|part| part["id"] != serde_json::json!(text_part_id))
        .map(|part| part["id"].as_str().expect("id").to_owned())
        .collect();

    // PATCH changes only the targeted part.
    let response = send(
        &app,
        "PATCH",
        &format!(
            "/session/{}/message/{}/part/{}",
            session.id, message_id, text_part_id
        ),
        Some(r#"{"type":"text","text":"rewritten text part"}"#),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let updated: serde_json::Value = json_body_large(response).await;
    assert_eq!(updated["type"], "text");
    assert_eq!(updated["text"], "rewritten text part");

    let response = send(
        &app,
        "GET",
        &format!("/session/{}/message/{}", session.id, message_id),
        None,
    )
    .await;
    let message: serde_json::Value = json_body_large(response).await;
    let reloaded_text = message["parts"]
        .as_array()
        .expect("parts")
        .iter()
        .find(|part| part["id"] == serde_json::json!(text_part_id))
        .expect("text part still present");
    assert_eq!(reloaded_text["text"], "rewritten text part");

    // DELETE removes only the targeted part.
    let response = send(
        &app,
        "DELETE",
        &format!(
            "/session/{}/message/{}/part/{}",
            session.id, message_id, text_part_id
        ),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let response = send(
        &app,
        "GET",
        &format!("/session/{}/message/{}", session.id, message_id),
        None,
    )
    .await;
    let message: serde_json::Value = json_body_large(response).await;
    let remaining: Vec<String> = message["parts"]
        .as_array()
        .expect("parts")
        .iter()
        .map(|part| part["id"].as_str().expect("id").to_owned())
        .collect();
    assert!(
        !remaining.contains(&text_part_id),
        "deleted part is gone: {remaining:?}"
    );
    for other in &other_ids {
        assert!(remaining.contains(other), "sibling part {other} survives");
    }

    // Unknown message and part are clean 404s.
    let response = send(
        &app,
        "GET",
        &format!("/session/{}/message/missing-id", session.id),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let response = send(
        &app,
        "DELETE",
        &format!(
            "/session/{}/message/{}/part/missing-part",
            session.id, message_id
        ),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    // DELETE the whole message removes it from the list.
    let response = send(
        &app,
        "DELETE",
        &format!("/session/{}/message/{}", session.id, message_id),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let response = send(
        &app,
        "GET",
        &format!("/session/{}/message", session.id),
        None,
    )
    .await;
    let body: serde_json::Value = json_body_large(response).await;
    assert!(
        body["messages"]
            .as_array()
            .expect("messages")
            .iter()
            .all(|message| message["id"] != serde_json::json!(message_id)),
        "deleted message no longer appears"
    );
}

#[tokio::test]
async fn status_wait_and_context_track_session_state() {
    let scratch = Scratch::new("server-status-wait");
    let provider = Arc::new(EchoProvider::with_delay(Duration::from_millis(800)));
    let app = server::app_with_provider(scratch.db_path(), provider);
    let session = create_session(&app).await;

    // Idle session reports idle.
    let response = send(
        &app,
        "GET",
        &format!("/session/{}/status", session.id),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value = json_body(response).await;
    assert_eq!(body["status"], "idle");
    assert_eq!(body["busy"], false);

    // Admit a background turn; observe the running state.
    let response = send(
        &app,
        "POST",
        &format!("/session/{}/prompt_async", session.id),
        Some(r#"{"text":"slow turn please"}"#),
    )
    .await;
    assert_eq!(response.status(), StatusCode::ACCEPTED);

    let mut saw_running = false;
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        let response = send(
            &app,
            "GET",
            &format!("/session/{}/status", session.id),
            None,
        )
        .await;
        let body: serde_json::Value = json_body(response).await;
        if body["busy"] == serde_json::json!(true) || body["status"] == serde_json::json!("running")
        {
            saw_running = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(saw_running, "status should report running during the turn");

    // wait blocks until the turn completes, then reports idle.
    let response = send(&app, "GET", &format!("/session/{}/wait", session.id), None).await;
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value = json_body(response).await;
    assert_eq!(body["status"], "idle");
    assert_eq!(body["busy"], false);

    // context reports model/agent/token estimate without a network call.
    let response = send(
        &app,
        "GET",
        &format!("/session/{}/context", session.id),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value = json_body(response).await;
    assert!(
        body["model"]
            .as_str()
            .is_some_and(|model| !model.is_empty())
    );
    assert!(
        body["agent"]
            .as_str()
            .is_some_and(|agent| !agent.is_empty())
    );
    assert!(
        body["messageCount"]
            .as_u64()
            .is_some_and(|count| count >= 2)
    );
}

#[tokio::test]
async fn prompt_async_admits_without_blocking_and_persists_answer() {
    let scratch = Scratch::new("server-prompt-async");
    let provider = Arc::new(EchoProvider::with_delay(Duration::from_millis(700)));
    let app = server::app_with_provider(scratch.db_path(), provider);
    let session = create_session(&app).await;

    let nonce = "async-nonce-4242";
    let started = Instant::now();
    let response = send(
        &app,
        "POST",
        &format!("/session/{}/prompt_async", session.id),
        Some(&format!(r#"{{"text":"echo {nonce}"}}"#)),
    )
    .await;
    let elapsed = started.elapsed();
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let body: serde_json::Value = json_body(response).await;
    assert_eq!(body["status"], "accepted");
    assert!(
        body["operationId"]
            .as_str()
            .is_some_and(|id| !id.is_empty()),
        "async response carries an operation id: {body:?}"
    );
    assert!(
        elapsed < Duration::from_millis(500),
        "async admission returns before the {}ms turn completes (took {elapsed:?})",
        700
    );

    // Final session contains the streamed answer once the background turn finishes.
    let mut answered = false;
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        let response = send(
            &app,
            "GET",
            &format!("/session/{}/message", session.id),
            None,
        )
        .await;
        let body: serde_json::Value = json_body_large(response).await;
        let messages = body["messages"].as_array().cloned().unwrap_or_default();
        if messages.iter().any(|message| {
            message["role"] == "assistant"
                && message["content"]
                    .as_str()
                    .is_some_and(|text| text.contains(nonce))
        }) {
            answered = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert!(
        answered,
        "background turn eventually persisted an assistant answer containing the nonce"
    );
}

#[tokio::test]
async fn fork_children_and_summarize_compact_are_end_to_end() {
    let scratch = Scratch::new("server-fork-summarize");
    let app = server::app_with_provider(scratch.db_path(), Arc::new(EchoProvider::default()));
    let parent = create_session(&app).await;
    for index in 0..2 {
        let response = send(
            &app,
            "POST",
            &format!("/session/{}/message", parent.id),
            Some(&format!(r#"{{"text":"parent turn {index}"}}"#)),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
    }
    let parent_messages = message_contents(&app, &parent.id).await;

    // Fork inherits the transcript and registers as a child.
    let response = send(
        &app,
        "POST",
        &format!("/session/{}/fork", parent.id),
        Some("{}"),
    )
    .await;
    assert_eq!(response.status(), StatusCode::CREATED);
    let child: serde_json::Value = json_body(response).await;
    let child_id = child["id"].as_str().expect("child id").to_owned();
    assert_eq!(child["parent_id"], serde_json::json!(parent.id));
    assert_eq!(
        message_contents(&app, &child_id).await,
        parent_messages,
        "forked child inherits the parent transcript"
    );

    let response = send(
        &app,
        "GET",
        &format!("/session/{}/children", parent.id),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let children: serde_json::Value = json_body(response).await;
    assert!(
        children
            .as_array()
            .expect("children")
            .iter()
            .any(|session| session["id"] == serde_json::json!(child_id)),
        "children route lists the forked session"
    );

    // Summarize produces a visible assistant summary message.
    let response = send(
        &app,
        "POST",
        &format!("/session/{}/summarize", parent.id),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value = json_body(response).await;
    assert_eq!(body["status"], "summarized");
    assert!(
        body["summary"]
            .as_str()
            .is_some_and(|text| !text.is_empty())
    );

    // Compact records a first-class compaction part plus a visible summary.
    let response = send(
        &app,
        "POST",
        &format!("/session/{}/compact", parent.id),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value = json_body(response).await;
    assert_eq!(body["status"], "compacted");
    let has_compaction = body["message"]["parts"]
        .as_array()
        .expect("parts")
        .iter()
        .any(|part| part["type"] == "compaction");
    assert!(
        has_compaction,
        "compact persists a compaction part: {body:?}"
    );

    // Summary/compaction messages survive a restart.
    let relaunched = server::app_with_db_path(scratch.db_path());
    let response = send(
        &relaunched,
        "GET",
        &format!("/session/{}/message", parent.id),
        None,
    )
    .await;
    let body: serde_json::Value = json_body_large(response).await;
    let assistant_count = body["messages"]
        .as_array()
        .expect("messages")
        .iter()
        .filter(|message| message["role"] == "assistant")
        .count();
    assert!(
        assistant_count >= 4,
        "two turns plus summarize and compact persist assistant messages: {assistant_count}"
    );
}

#[tokio::test]
async fn diff_revert_and_unrevert_change_disk_and_session_state() {
    let scratch = Scratch::new("server-diff-revert");
    let workspace = scratch.dir.join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace dir");
    let target = workspace.join("target.txt");
    let h0 = "H0 server checkpoint target\n";
    std::fs::write(&target, h0).expect("seed target");
    let checkpoint_base = scratch.dir.join("checkpoints");
    let provider = Arc::new(CheckpointEditingProvider::default());
    let state = server::ServerState::with_provider(scratch.db_path(), provider)
        .with_cwd(workspace.clone())
        .with_checkpoint_base(checkpoint_base.clone());
    let app = server::app_with_state(state);
    let session = create_session(&app).await;

    let response = send(
        &app,
        "POST",
        &format!("/session/{}/message", session.id),
        Some(r#"{"text":"edit target.txt from H0 to H1 using tools"}"#),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        std::fs::read_to_string(&target).expect("edited target"),
        "H1 server checkpoint target\n"
    );

    // Diff reports the modified file with H1 content.
    let response = send(&app, "GET", &format!("/session/{}/diff", session.id), None).await;
    assert_eq!(response.status(), StatusCode::OK);
    let diffs: serde_json::Value = json_body_large(response).await;
    let entry = diffs
        .as_array()
        .expect("diffs")
        .iter()
        .find(|entry| {
            entry["path"]
                .as_str()
                .is_some_and(|path| path.contains("target.txt"))
        })
        .expect("target.txt diff entry");
    assert_eq!(entry["status"], "modified");
    assert!(
        entry["diff"]
            .as_str()
            .is_some_and(|diff| diff.contains("H1")),
        "diff text reflects the net change: {entry:?}"
    );

    // Revert restores H0 on disk and marks the session reverted.
    let response = send(
        &app,
        "POST",
        &format!("/session/{}/revert", session.id),
        Some("{}"),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value = json_body(response).await;
    assert_eq!(body["status"], "reverted");
    assert!(!body["session"]["reverted_message_id"].is_null());
    assert_eq!(
        std::fs::read_to_string(&target).expect("reverted target"),
        h0,
        "revert restores the pre-edit bytes"
    );

    // Unrevert reapplies H1 and clears the reverted state.
    let response = send(
        &app,
        "POST",
        &format!("/session/{}/unrevert", session.id),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value = json_body(response).await;
    assert_eq!(body["status"], "unreverted");
    assert!(body["session"]["reverted_message_id"].is_null());
    assert_eq!(
        std::fs::read_to_string(&target).expect("unreverted target"),
        "H1 server checkpoint target\n",
        "unrevert reapplies the session changes"
    );
}

// ---- auxiliary turn admission (summarize/compact route through the shared TurnQueue) ----

// A prompt turn and an auxiliary summary for the same session serialize through the shared
// TurnQueue: the provider never observes two concurrent in-flight turns.
#[tokio::test]
async fn summarize_serializes_with_a_concurrent_prompt_turn() {
    let scratch = Scratch::new("server-aux-serialize");
    let provider = Arc::new(ConcurrencyProvider::new(Duration::from_millis(250)));
    let app = server::app_with_provider(scratch.db_path(), provider.clone());
    let session = create_session(&app).await;

    let message = {
        let app = app.clone();
        let id = session.id.clone();
        tokio::spawn(async move {
            send(
                &app,
                "POST",
                &format!("/session/{id}/message"),
                Some(r#"{"text":"hello"}"#),
            )
            .await
        })
    };
    let summarize = {
        let app = app.clone();
        let id = session.id.clone();
        tokio::spawn(
            async move { send(&app, "POST", &format!("/session/{id}/summarize"), None).await },
        )
    };

    let message = message.await.expect("message task");
    let summarize = summarize.await.expect("summarize task");
    assert_eq!(message.status(), StatusCode::OK);
    assert_eq!(summarize.status(), StatusCode::OK);
    assert_eq!(
        provider.max_in_flight(),
        1,
        "the prompt turn and the auxiliary summary must not run concurrently"
    );
    assert_eq!(provider.calls(), 2, "both turns reached the provider");
}

// While an auxiliary summary runs it is reflected as busy/running in /status and returns to idle
// once it completes.
#[tokio::test]
async fn summarize_is_busy_visible_in_status_while_running() {
    let scratch = Scratch::new("server-aux-busy");
    let provider = Arc::new(ConcurrencyProvider::new(Duration::from_millis(600)));
    let app = server::app_with_provider(scratch.db_path(), provider.clone());
    let session = create_session(&app).await;

    let summarize = {
        let app = app.clone();
        let id = session.id.clone();
        tokio::spawn(
            async move { send(&app, "POST", &format!("/session/{id}/summarize"), None).await },
        )
    };

    let mut saw_busy = false;
    for _ in 0..60 {
        let status: serde_json::Value = json_body(
            send(
                &app,
                "GET",
                &format!("/session/{}/status", session.id),
                None,
            )
            .await,
        )
        .await;
        if status["busy"] == serde_json::json!(true) {
            assert_eq!(status["status"], "running", "busy aux turn reports running");
            saw_busy = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(
        saw_busy,
        "the summarize turn should show as busy in /status"
    );

    let summarize = summarize.await.expect("summarize task");
    assert_eq!(summarize.status(), StatusCode::OK);

    let status: serde_json::Value = json_body(
        send(
            &app,
            "GET",
            &format!("/session/{}/status", session.id),
            None,
        )
        .await,
    )
    .await;
    assert_eq!(
        status["busy"],
        serde_json::json!(false),
        "the session is idle once the aux turn finishes"
    );
}

// An in-flight auxiliary summary is abortable via POST /session/{id}/abort: the turn is cancelled,
// returns a conflict, and persists no assistant message.
#[tokio::test]
async fn summarize_turn_is_abortable() {
    let scratch = Scratch::new("server-aux-abort");
    let provider = Arc::new(ConcurrencyProvider::new(Duration::from_secs(5)));
    let app = server::app_with_provider(scratch.db_path(), provider.clone());
    let session = create_session(&app).await;

    let summarize = {
        let app = app.clone();
        let id = session.id.clone();
        tokio::spawn(
            async move { send(&app, "POST", &format!("/session/{id}/summarize"), None).await },
        )
    };

    let mut aborted = false;
    for _ in 0..60 {
        let status: serde_json::Value = json_body(
            send(
                &app,
                "GET",
                &format!("/session/{}/status", session.id),
                None,
            )
            .await,
        )
        .await;
        if status["busy"] == serde_json::json!(true) {
            let abort: serde_json::Value = json_body(
                send(
                    &app,
                    "POST",
                    &format!("/session/{}/abort", session.id),
                    None,
                )
                .await,
            )
            .await;
            assert_eq!(
                abort["active"],
                serde_json::json!(true),
                "abort signaled the in-flight aux turn"
            );
            aborted = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(
        aborted,
        "the summarize turn should have been in-flight to abort"
    );

    let summarize = summarize.await.expect("summarize task");
    assert_eq!(
        summarize.status(),
        StatusCode::CONFLICT,
        "an aborted aux turn returns a conflict, not a summary"
    );

    let messages: serde_json::Value = json_body_large(
        send(
            &app,
            "GET",
            &format!("/session/{}/message", session.id),
            None,
        )
        .await,
    )
    .await;
    let assistant = messages["messages"]
        .as_array()
        .expect("messages")
        .iter()
        .filter(|message| message["role"] == "assistant")
        .count();
    assert_eq!(
        assistant, 0,
        "an aborted summarize must not persist an assistant message"
    );
}

// `/compact` publishes the `session.compacted` SSE event and persists a first-class compaction
// part, matching the auto-compaction path so an explicit compaction streams like automatic one.
#[tokio::test]
async fn compact_publishes_session_compacted_event_and_compaction_part() {
    let scratch = Scratch::new("server-aux-compact-sse");
    let bus = Arc::new(events::EventBus::new());
    let collector = Arc::new(EventCollector::default());
    bus.subscribe(collector.clone());
    let state =
        server::ServerState::with_provider(scratch.db_path(), Arc::new(EchoProvider::default()))
            .with_event_bus(bus.clone());
    let app = server::app_with_state(state);
    let session = create_session(&app).await;
    for index in 0..2 {
        let response = send(
            &app,
            "POST",
            &format!("/session/{}/message", session.id),
            Some(&format!(r#"{{"text":"turn {index}"}}"#)),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
    }

    let response = send(
        &app,
        "POST",
        &format!("/session/{}/compact", session.id),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value = json_body(response).await;
    assert_eq!(body["status"], "compacted");
    let has_compaction = body["message"]["parts"]
        .as_array()
        .expect("parts")
        .iter()
        .any(|part| part["type"] == "compaction");
    assert!(
        has_compaction,
        "compact persists a compaction part: {body:?}"
    );

    assert!(
        collector.kinds().contains(&"session.compacted".to_owned()),
        "compact should publish session.compacted like auto-compaction: {:?}",
        collector.kinds()
    );
}

// `/revert` is session-aware: it refuses to undo a turn recorded by a different session, so one
// session cannot revert another session's most-recent turn off the workspace-global undo stack.
#[tokio::test]
async fn revert_is_session_aware_across_interleaved_sessions() {
    let scratch = Scratch::new("server-revert-multi");
    let workspace = scratch.dir.join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace dir");
    std::fs::write(workspace.join("alpha.txt"), "v0\n").expect("seed alpha");
    std::fs::write(workspace.join("beta.txt"), "v0\n").expect("seed beta");
    let checkpoint_base = scratch.dir.join("checkpoints");
    let provider = Arc::new(MarkerEditingProvider);
    let state = server::ServerState::with_provider(scratch.db_path(), provider)
        .with_cwd(workspace.clone())
        .with_checkpoint_base(checkpoint_base.clone());
    let app = server::app_with_state(state);

    let session_a = create_session(&app).await;
    let session_b = create_session(&app).await;

    // Session A edits alpha.txt; session B then edits beta.txt, so B's turn is the most-recent
    // applied turn on the shared, workspace-global undo stack.
    let resp = send(
        &app,
        "POST",
        &format!("/session/{}/message", session_a.id),
        Some(r#"{"text":"edit alpha"}"#),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(
        std::fs::read_to_string(workspace.join("alpha.txt")).unwrap(),
        "v1\n",
        "session A edited alpha.txt"
    );
    let resp = send(
        &app,
        "POST",
        &format!("/session/{}/message", session_b.id),
        Some(r#"{"text":"edit beta"}"#),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(
        std::fs::read_to_string(workspace.join("beta.txt")).unwrap(),
        "v1\n",
        "session B edited beta.txt"
    );

    // Session A must NOT revert B's most-recent turn.
    let refused = send(
        &app,
        "POST",
        &format!("/session/{}/revert", session_a.id),
        Some("{}"),
    )
    .await;
    assert_eq!(
        refused.status(),
        StatusCode::CONFLICT,
        "A cannot revert B's most-recent turn"
    );
    assert_eq!(
        std::fs::read_to_string(workspace.join("beta.txt")).unwrap(),
        "v1\n",
        "B's file is untouched by A's refused revert"
    );

    // Session B reverts its own (top) turn.
    let reverted = send(
        &app,
        "POST",
        &format!("/session/{}/revert", session_b.id),
        Some("{}"),
    )
    .await;
    assert_eq!(reverted.status(), StatusCode::OK);
    let body: serde_json::Value = json_body(reverted).await;
    assert_eq!(body["status"], "reverted");
    assert_eq!(
        std::fs::read_to_string(workspace.join("beta.txt")).unwrap(),
        "v0\n",
        "B's revert restored beta.txt"
    );

    // Now A's turn is on top and A can revert it.
    let reverted = send(
        &app,
        "POST",
        &format!("/session/{}/revert", session_a.id),
        Some("{}"),
    )
    .await;
    assert_eq!(reverted.status(), StatusCode::OK);
    let body: serde_json::Value = json_body(reverted).await;
    assert_eq!(body["status"], "reverted");
    assert_eq!(
        std::fs::read_to_string(workspace.join("alpha.txt")).unwrap(),
        "v0\n",
        "A's revert restored alpha.txt"
    );
}

// ---- helpers ----

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

async fn create_session(app: &axum::Router) -> Session {
    let response = send(app, "POST", "/session", Some("{}")).await;
    assert_eq!(response.status(), StatusCode::CREATED);
    json_body(response).await
}

async fn message_contents(app: &axum::Router, session_id: &str) -> Vec<String> {
    let response = send(app, "GET", &format!("/session/{session_id}/message"), None).await;
    let body: serde_json::Value = json_body_large(response).await;
    body["messages"]
        .as_array()
        .expect("messages")
        .iter()
        .map(|message| message["content"].as_str().unwrap_or_default().to_owned())
        .collect()
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
    let bytes = to_bytes(response.into_body(), 262_144).await.expect("body");
    serde_json::from_slice(&bytes).expect("JSON body")
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
            let events = vec![
                LlmEvent::TextDelta {
                    text: format!("assistant saw: {prompt}"),
                },
                LlmEvent::Usage(Usage {
                    input_tokens: 11,
                    output_tokens: 7,
                    total_tokens: 18,
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
struct RichPartsProvider {
    calls: AtomicUsize,
}

impl Provider for RichPartsProvider {
    fn stream<'a>(&'a self, _request: LlmRequest) -> BoxFuture<'a, Result<LlmStream, LlmError>> {
        Box::pin(async move {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let events = vec![
                LlmEvent::Reasoning {
                    text: "thinking about parts".to_owned(),
                    metadata: None,
                },
                LlmEvent::TextDelta {
                    text: "final text part".to_owned(),
                },
                LlmEvent::Usage(Usage {
                    input_tokens: 9,
                    output_tokens: 4,
                    total_tokens: 13,
                    cache_creation_input_tokens: None,
                    cache_read_input_tokens: None,
                    reasoning_output_tokens: Some(2),
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
struct CheckpointEditingProvider {
    calls: AtomicUsize,
}

impl Provider for CheckpointEditingProvider {
    fn stream<'a>(&'a self, _request: LlmRequest) -> BoxFuture<'a, Result<LlmStream, LlmError>> {
        Box::pin(async move {
            let call = self.calls.fetch_add(1, Ordering::SeqCst);
            let events = match call {
                0 => vec![
                    LlmEvent::ToolCall(ToolCall {
                        id: "call_read".to_owned(),
                        name: "read".to_owned(),
                        arguments: r#"{"path":"target.txt"}"#.to_owned(),
                    }),
                    LlmEvent::Done {
                        reason: FinishReason::ToolCalls,
                    },
                ],
                1 => vec![
                    LlmEvent::ToolCall(ToolCall {
                        id: "call_edit".to_owned(),
                        name: "edit".to_owned(),
                        arguments: serde_json::json!({
                            "path": "target.txt",
                            "oldString": "H0 server checkpoint target\n",
                            "newString": "H1 server checkpoint target\n"
                        })
                        .to_string(),
                    }),
                    LlmEvent::Done {
                        reason: FinishReason::ToolCalls,
                    },
                ],
                _ => vec![
                    LlmEvent::TextDelta {
                        text: "server edit complete".to_owned(),
                    },
                    LlmEvent::Usage(Usage {
                        input_tokens: 13,
                        output_tokens: 5,
                        total_tokens: 18,
                        cache_creation_input_tokens: None,
                        cache_read_input_tokens: None,
                        reasoning_output_tokens: None,
                        cost: Some(0.000_001),
                    }),
                    LlmEvent::Done {
                        reason: FinishReason::Stop,
                    },
                ],
            };
            Ok(Box::pin(futures_util::stream::iter(events)) as LlmStream)
        })
    }
}

struct ConcurrencyProvider {
    delay: Duration,
    in_flight: AtomicUsize,
    max_in_flight: AtomicUsize,
    calls: AtomicUsize,
}

impl ConcurrencyProvider {
    fn new(delay: Duration) -> Self {
        Self {
            delay,
            in_flight: AtomicUsize::new(0),
            max_in_flight: AtomicUsize::new(0),
            calls: AtomicUsize::new(0),
        }
    }

    fn max_in_flight(&self) -> usize {
        self.max_in_flight.load(Ordering::SeqCst)
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl Provider for ConcurrencyProvider {
    fn stream<'a>(&'a self, request: LlmRequest) -> BoxFuture<'a, Result<LlmStream, LlmError>> {
        Box::pin(async move {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let now = self.in_flight.fetch_add(1, Ordering::SeqCst) + 1;
            self.max_in_flight.fetch_max(now, Ordering::SeqCst);
            tokio::time::sleep(self.delay).await;
            self.in_flight.fetch_sub(1, Ordering::SeqCst);
            let prompt = request
                .messages
                .iter()
                .rev()
                .find(|message| message.role == Role::User)
                .map(|message| message.content.clone())
                .unwrap_or_default();
            let events = vec![
                LlmEvent::TextDelta {
                    text: format!("aux saw: {prompt}"),
                },
                LlmEvent::Usage(Usage {
                    input_tokens: 7,
                    output_tokens: 3,
                    total_tokens: 10,
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

// For each session, reads then edits the file named by the last word of the user prompt, rewriting
// "v0\n" to "v1\n" (the `edit` tool requires a prior read of the file), then finishes once both tool
// results return. Gives each session its own checkpointed turn against a distinct file.
#[derive(Debug)]
struct MarkerEditingProvider;

impl Provider for MarkerEditingProvider {
    fn stream<'a>(&'a self, request: LlmRequest) -> BoxFuture<'a, Result<LlmStream, LlmError>> {
        Box::pin(async move {
            let tool_rounds = request
                .messages
                .iter()
                .filter(|message| message.role == Role::Tool)
                .count();
            let prompt = request
                .messages
                .iter()
                .rev()
                .find(|message| message.role == Role::User)
                .map(|message| message.content.clone())
                .unwrap_or_default();
            let marker = prompt
                .split_whitespace()
                .last()
                .unwrap_or("file")
                .to_owned();
            let path = format!("{marker}.txt");
            let events = match tool_rounds {
                0 => vec![
                    LlmEvent::ToolCall(ToolCall {
                        id: "call_read".to_owned(),
                        name: "read".to_owned(),
                        arguments: serde_json::json!({ "path": path }).to_string(),
                    }),
                    LlmEvent::Done {
                        reason: FinishReason::ToolCalls,
                    },
                ],
                1 => vec![
                    LlmEvent::ToolCall(ToolCall {
                        id: "call_edit".to_owned(),
                        name: "edit".to_owned(),
                        arguments: serde_json::json!({
                            "path": path,
                            "oldString": "v0\n",
                            "newString": "v1\n"
                        })
                        .to_string(),
                    }),
                    LlmEvent::Done {
                        reason: FinishReason::ToolCalls,
                    },
                ],
                _ => vec![
                    LlmEvent::TextDelta {
                        text: "edit complete".to_owned(),
                    },
                    LlmEvent::Usage(Usage {
                        input_tokens: 9,
                        output_tokens: 4,
                        total_tokens: 13,
                        cache_creation_input_tokens: None,
                        cache_read_input_tokens: None,
                        reasoning_output_tokens: None,
                        cost: Some(0.000_001),
                    }),
                    LlmEvent::Done {
                        reason: FinishReason::Stop,
                    },
                ],
            };
            Ok(Box::pin(futures_util::stream::iter(events)) as LlmStream)
        })
    }
}

#[derive(Default)]
struct EventCollector {
    kinds: Mutex<Vec<String>>,
}

impl EventCollector {
    fn kinds(&self) -> Vec<String> {
        self.kinds.lock().expect("collector lock").clone()
    }
}

impl events::Subscriber for EventCollector {
    fn on_event(&self, event: &events::Event) {
        self.kinds
            .lock()
            .expect("collector lock")
            .push(event.kind().to_owned());
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
