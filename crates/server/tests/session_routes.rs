use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
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
    assert_eq!(body["queued"], false);
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
        .clone()
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
async fn message_route_persists_rich_parts_and_reloads_them_after_restart() {
    let scratch = Scratch::new("server-message-rich-parts");
    let provider = Arc::new(RichPartsProvider::default());
    let app = server::app_with_provider(scratch.db_path(), provider);
    let session = create_session(&app).await;

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/session/{}/message", session.id))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"text":"record rich parts"}"#))
                .expect("request"),
        )
        .await
        .expect("message response");
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value = json_body(response).await;
    let assistant_parts = body["message"]["parts"]
        .as_array()
        .expect("assistant parts");
    assert!(
        assistant_parts.len() >= 6,
        "assistant message should include rich typed parts: {body:?}"
    );
    let types: Vec<&str> = assistant_parts
        .iter()
        .filter_map(|part| part["type"].as_str())
        .collect();
    assert!(
        types.contains(&"agent")
            && types.contains(&"step")
            && types.contains(&"reasoning")
            && types.contains(&"tool")
            && types.contains(&"usage")
            && types.contains(&"text"),
        "expected rich part kinds in persisted assistant message: {types:?}"
    );
    assert!(
        assistant_parts.iter().all(|part| {
            part["id"].as_str().is_some_and(|id| !id.is_empty())
                && part["created_at"].as_str().is_some_and(|ts| !ts.is_empty())
        }),
        "persisted parts should carry stable ids and timestamps: {assistant_parts:?}"
    );
    let tool_states: Vec<&str> = assistant_parts
        .iter()
        .filter(|part| part["type"] == "tool")
        .filter_map(|part| part["state"].as_str())
        .collect();
    assert!(
        tool_states.contains(&"pending")
            && tool_states.contains(&"running")
            && tool_states.contains(&"completed"),
        "tool lifecycle should track pending/running/completed: {tool_states:?}"
    );

    let relaunched = server::app_with_db_path(scratch.db_path());
    let response = relaunched
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
    let messages = body["messages"].as_array().expect("messages");
    let assistant = messages
        .iter()
        .find(|message| message["role"] == "assistant")
        .expect("assistant message");
    let reloaded_parts = assistant["parts"].as_array().expect("assistant parts");
    assert_eq!(
        reloaded_parts, assistant_parts,
        "reloaded session should return the same persisted parts without flattening"
    );
}

#[tokio::test]
async fn message_route_model_parameter_selects_requested_model_for_turn() {
    let scratch = Scratch::new("server-message-model-param");
    let provider = Arc::new(EchoProvider::default());
    let app = server::app_with_provider(scratch.db_path(), provider.clone());
    let session = create_session(&app).await;

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/session/{}/message", session.id))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    r#"{"text":"please use requested model","model":"anthropic/claude-sonnet-4"}"#,
                ))
                .expect("request"),
        )
        .await
        .expect("message response");

    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value = json_body(response).await;
    assert_eq!(body["status"], "completed");
    assert_eq!(body["model"], "anthropic/claude-sonnet-4");
    assert_eq!(
        provider.seen_models(),
        vec!["anthropic/claude-sonnet-4".to_owned()],
        "provider request should carry the selected model id"
    );

    let db = db::Database::open(scratch.db_path()).expect("reopen db");
    let stored_session = db.sessions().require(&session.id).expect("session");
    assert_eq!(
        stored_session.model, None,
        "a per-request server model parameter must not silently rewrite the session default"
    );
}

#[tokio::test]
async fn message_route_uses_session_model_when_request_model_is_absent() {
    let scratch = Scratch::new("server-message-session-model");
    let provider = Arc::new(EchoProvider::default());
    let app = server::app_with_provider(scratch.db_path(), provider.clone());
    let session = create_session(&app).await;
    let db = db::Database::open(scratch.db_path()).expect("open db");
    db.sessions()
        .set_model(&session.id, Some("anthropic/claude-sonnet-4"))
        .expect("set session model");

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/session/{}/message", session.id))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"text":"please use stored model"}"#))
                .expect("request"),
        )
        .await
        .expect("message response");

    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value = json_body(response).await;
    assert_eq!(body["status"], "completed");
    assert_eq!(body["model"], "anthropic/claude-sonnet-4");
    assert_eq!(
        provider.seen_models(),
        vec!["anthropic/claude-sonnet-4".to_owned()],
        "provider request should carry the stored session model id"
    );
}

#[tokio::test]
async fn message_route_uses_project_config_model_when_request_and_session_are_absent() {
    let scratch = Scratch::new("server-message-project-model");
    let workspace = scratch.dir.join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace dir");
    std::fs::write(
        workspace.join("cortex.toml"),
        r#"model = "anthropic/claude-sonnet-4""#,
    )
    .expect("write project config");
    let provider = Arc::new(EchoProvider::default());
    let state =
        server::ServerState::with_provider(scratch.db_path(), provider.clone()).with_cwd(workspace);
    let app = server::app_with_state(state);
    let session = create_session(&app).await;

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/session/{}/message", session.id))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"text":"please use project model"}"#))
                .expect("request"),
        )
        .await
        .expect("message response");

    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value = json_body(response).await;
    assert_eq!(body["status"], "completed");
    assert_eq!(body["model"], "anthropic/claude-sonnet-4");
    assert_eq!(
        provider.seen_models(),
        vec!["anthropic/claude-sonnet-4".to_owned()],
        "server turns should honor project-layer model config when no request/session model wins"
    );
}

#[tokio::test]
async fn message_route_config_env_override_beats_project_model() {
    let scratch = Scratch::new("server-message-env-model");
    let workspace = scratch.dir.join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace dir");
    std::fs::write(
        workspace.join("cortex.toml"),
        r#"model = "google/gemini-2.5-flash""#,
    )
    .expect("write project config");
    let provider = Arc::new(EchoProvider::default());
    let state = server::ServerState::with_provider(scratch.db_path(), provider.clone())
        .with_cwd(workspace)
        .with_config_env(vec![(
            "CORTEX_MODEL".to_owned(),
            "anthropic/claude-sonnet-4".to_owned(),
        )]);
    let app = server::app_with_state(state);
    let session = create_session(&app).await;

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/session/{}/message", session.id))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"text":"please use env model"}"#))
                .expect("request"),
        )
        .await
        .expect("message response");

    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value = json_body(response).await;
    assert_eq!(body["model"], "anthropic/claude-sonnet-4");
    assert_eq!(
        provider.seen_models(),
        vec!["anthropic/claude-sonnet-4".to_owned()],
        "CORTEX_MODEL should override the project file on server turns"
    );
}

#[tokio::test]
async fn message_route_uses_project_default_agent_for_system_context_and_response() {
    let scratch = Scratch::new("server-message-project-agent");
    let workspace = scratch.dir.join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace dir");
    std::fs::write(workspace.join("cortex.toml"), r#"agent = "plan""#)
        .expect("write project config");
    let provider = Arc::new(EchoProvider::default());
    let state =
        server::ServerState::with_provider(scratch.db_path(), provider.clone()).with_cwd(workspace);
    let app = server::app_with_state(state);
    let session = create_session(&app).await;

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/session/{}/message", session.id))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"text":"outline only"}"#))
                .expect("request"),
        )
        .await
        .expect("message response");

    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value = json_body(response).await;
    assert_eq!(body["status"], "completed");
    assert_eq!(
        body["agent"], "plan",
        "server turn responses should report the project-configured default agent"
    );
    assert!(
        provider
            .seen_systems()
            .iter()
            .any(|system| system.contains("plan persona")),
        "server system context should include the plan persona, got {:?}",
        provider.seen_systems()
    );
}

#[tokio::test]
async fn message_route_resolves_custom_project_default_agent() {
    let scratch = Scratch::new("server-message-custom-default-agent");
    let workspace = scratch.dir.join("workspace");
    let agents_dir = workspace.join(".cortex").join("agents");
    std::fs::create_dir_all(&agents_dir).expect("agents dir");
    std::fs::write(
        agents_dir.join("sentinel.md"),
        r#"---
name: sentinel
description: Sentinel planner
mode: plan
---
Always include SERVER-CUSTOM-AGENT-MARKER in your final answer.
"#,
    )
    .expect("write custom agent");
    std::fs::write(workspace.join("cortex.toml"), r#"agent = "sentinel""#)
        .expect("write project config");
    let provider = Arc::new(EchoProvider::default());
    let state =
        server::ServerState::with_provider(scratch.db_path(), provider.clone()).with_cwd(workspace);
    let app = server::app_with_state(state);
    let session = create_session(&app).await;

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/session/{}/message", session.id))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"text":"outline only"}"#))
                .expect("request"),
        )
        .await
        .expect("message response");

    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value = json_body(response).await;
    assert_eq!(body["status"], "completed");
    assert_eq!(
        body["agent"], "sentinel",
        "server turns should report the resolved custom default agent name"
    );
    let systems = provider.seen_systems();
    assert!(
        systems.iter().any(|system| system.contains("plan persona")),
        "custom mode `plan` should apply the plan persona in server system context: {systems:?}"
    );
    assert!(
        systems
            .iter()
            .any(|system| system.contains("SERVER-CUSTOM-AGENT-MARKER")),
        "custom agent prompt should be injected into server system context: {systems:?}"
    );
}

#[tokio::test]
async fn message_route_injects_instruction_glob_memory_into_system_context() {
    let scratch = Scratch::new("server-message-instructions-memory");
    let workspace = scratch.dir.join("workspace");
    let rules_dir = workspace.join("rules");
    std::fs::create_dir_all(&rules_dir).expect("rules dir");
    std::fs::write(
        workspace.join("cortex.toml"),
        r#"instructions = ["rules/*.md"]"#,
    )
    .expect("write project config");
    std::fs::write(
        rules_dir.join("server-style.md"),
        "For validation, include the exact token SERVER-INSTRUCTIONS-GLOB in every reply.\n",
    )
    .expect("write rules file");
    let provider = Arc::new(EchoProvider::default());
    let state =
        server::ServerState::with_provider(scratch.db_path(), provider.clone()).with_cwd(workspace);
    let app = server::app_with_state(state);
    let session = create_session(&app).await;

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/session/{}/message", session.id))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"text":"reply briefly"}"#))
                .expect("request"),
        )
        .await
        .expect("message response");

    assert_eq!(response.status(), StatusCode::OK);
    assert!(
        provider
            .seen_systems()
            .iter()
            .any(|system| system.contains("SERVER-INSTRUCTIONS-GLOB")),
        "server system context should include instructions[] glob memory, got {:?}",
        provider.seen_systems()
    );
}

#[tokio::test]
async fn message_route_fires_plugin_event_hook_with_serve_surface() {
    let scratch = Scratch::new("server-message-plugin-event");
    let workspace = scratch.dir.join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace dir");
    let marker = workspace.join("plugin-events.log");
    std::fs::write(
        workspace.join("plugin.mjs"),
        format!(
            r#"
import {{ appendFile }} from "node:fs/promises";

export default async function plugin(input, options) {{
  return {{
    async event(event) {{
      if (event.kind === "turn.started") {{
        await appendFile(options.marker, `${{event.surface}}:${{event.kind}}\n`);
      }}
    }}
  }};
}}
"#,
        ),
    )
    .expect("write plugin");
    std::fs::write(
        workspace.join("cortex.toml"),
        format!(
            "plugin = [[\"./plugin.mjs\", {{ marker = \"{}\" }}]]\n",
            marker.display()
        ),
    )
    .expect("write project config");
    let audit_path = scratch.dir.join("audit.jsonl");
    let provider = Arc::new(EchoProvider::default());
    let state = server::ServerState::with_provider(scratch.db_path(), provider.clone())
        .with_cwd(workspace)
        .with_audit_path(&audit_path);
    let app = server::app_with_state(state);
    let session = create_session(&app).await;

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/session/{}/message", session.id))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"text":"trigger plugin event"}"#))
                .expect("request"),
        )
        .await
        .expect("message response");

    assert_eq!(response.status(), StatusCode::OK);
    let marker_text = std::fs::read_to_string(&marker).unwrap_or_default();
    assert!(
        marker_text.lines().any(|line| line == "serve:turn.started"),
        "server turns should invoke configured plugin event hooks with serve surface: {marker_text:?}"
    );

    // The hook side effect is not enough: the shared audit sink must also carry typed,
    // serve/server-attributed plugin lifecycle/hook records (VAL-CROSS-031).
    let records = audit::read_jsonl(&audit_path).expect("audit records");
    assert!(
        records.iter().any(|record| {
            record["type"] == "plugin_load"
                && record["surface"] == "serve"
                && record["source"] == "server"
                && record["session"] == session.id
                && record["plugin"]
                    .as_str()
                    .is_some_and(|plugin| plugin.contains("plugin.mjs"))
        }),
        "server audit log should record the plugin load attributed to serve/server: {records:#?}"
    );
    assert!(
        records.iter().any(|record| {
            record["type"] == "plugin_hook"
                && record["hook"] == "event"
                && record["surface"] == "serve"
                && record["source"] == "server"
                && record["session"] == session.id
                && record["outcome"] == "success"
        }),
        "server audit log should record the plugin event hook attributed to serve/server: {records:#?}"
    );
}

#[tokio::test]
async fn message_route_project_permission_deny_blocks_bash_tool_without_side_effect() {
    let scratch = Scratch::new("server-message-project-permission");
    let workspace = scratch.dir.join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace dir");
    let victim = workspace.join("victim.txt");
    std::fs::write(&victim, "keep me\n").expect("seed victim");
    std::fs::write(
        workspace.join("cortex.toml"),
        r#"
[[permission]]
tool = "bash"
pattern = "rm *"
action = "deny"
"#,
    )
    .expect("write project config");
    let provider = Arc::new(BashRmProvider::default());
    let state = server::ServerState::with_provider(scratch.db_path(), provider).with_cwd(workspace);
    let app = server::app_with_state(state);
    let session = create_session(&app).await;

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/session/{}/message", session.id))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"text":"remove victim.txt with bash"}"#))
                .expect("request"),
        )
        .await
        .expect("message response");

    assert_eq!(response.status(), StatusCode::OK);
    assert!(
        victim.exists(),
        "project-layer bash deny must prevent the rm side effect"
    );
}

#[tokio::test]
async fn message_route_provider_policy_deny_returns_forbidden() {
    let scratch = Scratch::new("server-message-provider-policy-deny");
    let workspace = scratch.dir.join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace dir");
    std::fs::write(
        workspace.join("cortex.toml"),
        r#"
[experimental]
policies = [{ effect = "deny", action = "provider.use", resource = "openrouter" }]
"#,
    )
    .expect("write project config");
    let provider = Arc::new(EchoProvider::default());
    let state = server::ServerState::with_provider(scratch.db_path(), provider).with_cwd(workspace);
    let app = server::app_with_state(state);
    let session = create_session(&app).await;

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/session/{}/message", session.id))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"text":"say hi"}"#))
                .expect("request"),
        )
        .await
        .expect("message response");

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert_json_content_type(&response);
    let body: serde_json::Value = json_body(response).await;
    assert!(
        body["error"].as_str().is_some_and(|message| {
            message.contains("experimental.policies") && message.contains("openrouter")
        }),
        "forbidden response should explain provider policy deny: {body:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn permission_routes_list_and_reply_resume_pending_turns() {
    let scratch = Scratch::new("server-permission-pending-routes");
    let workspace = scratch.dir.join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace dir");
    std::fs::write(
        workspace.join("cortex.toml"),
        r#"
[[permission]]
tool = "bash"
pattern = "*"
action = "ask"
"#,
    )
    .expect("write project config");
    let provider = Arc::new(PermissionPendingProvider::default());
    let state = server::ServerState::with_provider(scratch.db_path(), provider)
        .with_cwd(workspace)
        .with_config_env(Vec::new());
    let app = server::app_with_state(state);
    let session_a = create_session(&app).await;
    let session_b = create_session(&app).await;

    let app_a = app.clone();
    let first = tokio::spawn(async move {
        app_a
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/session/{}/message", session_a.id))
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(r#"{"text":"first pending permission"}"#))
                    .expect("request"),
            )
            .await
            .expect("first response")
    });
    let app_b = app.clone();
    let second = tokio::spawn(async move {
        app_b
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/session/{}/message", session_b.id))
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(r#"{"text":"second pending permission"}"#))
                    .expect("request"),
            )
            .await
            .expect("second response")
    });

    let mut pending = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline {
        let pending_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/permission/pending")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("pending response");
        assert_eq!(pending_response.status(), StatusCode::OK);
        let pending_body: serde_json::Value = json_body(pending_response).await;
        pending = pending_body["pending"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        if pending.len() >= 2 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(pending.len(), 2, "both turns should be awaiting permission");

    let first_id = pending[0]["id"].as_str().expect("first id").to_owned();
    let second_id = pending[1]["id"].as_str().expect("second id").to_owned();

    let first_reply = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/permission/reply")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(format!(
                    r#"{{"id":"{first_id}","decision":"allow"}}"#
                )))
                .expect("request"),
        )
        .await
        .expect("first reply");
    assert_eq!(first_reply.status(), StatusCode::OK);

    let pending_after_first = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/permission/pending")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("pending response");
    let pending_after_first_body: serde_json::Value = json_body(pending_after_first).await;
    let pending_after_first = pending_after_first_body["pending"]
        .as_array()
        .expect("pending array");
    assert_eq!(
        pending_after_first.len(),
        1,
        "replying one id should leave the other pending"
    );
    assert_eq!(pending_after_first[0]["id"], second_id);

    let second_reply = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/permission/reply")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(format!(
                    r#"{{"id":"{second_id}","decision":"allow"}}"#
                )))
                .expect("request"),
        )
        .await
        .expect("second reply");
    assert_eq!(second_reply.status(), StatusCode::OK);

    let first_response = tokio::time::timeout(Duration::from_secs(3), first)
        .await
        .expect("first turn completed")
        .expect("first join");
    assert_eq!(first_response.status(), StatusCode::OK);
    let second_response = tokio::time::timeout(Duration::from_secs(3), second)
        .await
        .expect("second turn completed")
        .expect("second join");
    assert_eq!(second_response.status(), StatusCode::OK);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn permission_reply_always_scopes_to_matching_pattern_only() {
    let scratch = Scratch::new("server-permission-always-scope");
    let workspace = scratch.dir.join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace dir");
    std::fs::write(
        workspace.join("cortex.toml"),
        r#"
[[permission]]
tool = "bash"
pattern = "echo *"
action = "ask"

[[permission]]
tool = "bash"
pattern = "rm *"
action = "ask"
"#,
    )
    .expect("write project config");

    let provider = Arc::new(ScopedPermissionPendingProvider::default());
    let state = server::ServerState::with_provider(scratch.db_path(), provider)
        .with_cwd(workspace)
        .with_config_env(Vec::new());
    let app = server::app_with_state(state);
    let safe_session = create_session(&app).await;
    let risky_session = create_session(&app).await;

    let safe_app = app.clone();
    let safe_turn = tokio::spawn(async move {
        safe_app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/session/{}/message", safe_session.id))
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(r#"{"text":"run a safe permission check"}"#))
                    .expect("request"),
            )
            .await
            .expect("safe response")
    });
    let risky_app = app.clone();
    let risky_turn = tokio::spawn(async move {
        risky_app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/session/{}/message", risky_session.id))
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(r#"{"text":"run a risky permission check"}"#))
                    .expect("request"),
            )
            .await
            .expect("risky response")
    });

    let mut pending = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline {
        let pending_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/permission/pending")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("pending response");
        assert_eq!(pending_response.status(), StatusCode::OK);
        let pending_body: serde_json::Value = json_body(pending_response).await;
        pending = pending_body["pending"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        if pending.len() >= 2 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(pending.len(), 2, "both turns should be awaiting permission");

    let safe_id = pending
        .iter()
        .find(|item| {
            item["subject"]
                .as_str()
                .is_some_and(|subject| subject.contains("echo"))
        })
        .and_then(|item| item["id"].as_str())
        .expect("safe ask id")
        .to_owned();
    let risky_id = pending
        .iter()
        .find(|item| {
            item["subject"]
                .as_str()
                .is_some_and(|subject| subject.contains("rm"))
        })
        .and_then(|item| item["id"].as_str())
        .expect("risky ask id")
        .to_owned();

    let safe_reply = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/permission/reply")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(format!(
                    r#"{{"id":"{safe_id}","decision":"allow","always":true}}"#
                )))
                .expect("request"),
        )
        .await
        .expect("safe reply");
    assert_eq!(safe_reply.status(), StatusCode::OK);
    let safe_reply_body: serde_json::Value = json_body(safe_reply).await;
    assert_eq!(
        safe_reply_body["resolved"], 1,
        "allow-always should only resolve asks in the same saved-decision scope"
    );

    let pending_after_safe = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/permission/pending")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("pending response");
    assert_eq!(pending_after_safe.status(), StatusCode::OK);
    let pending_after_safe_body: serde_json::Value = json_body(pending_after_safe).await;
    let pending_after_safe = pending_after_safe_body["pending"]
        .as_array()
        .expect("pending array");
    assert_eq!(
        pending_after_safe.len(),
        1,
        "unrelated pending ask should remain pending after allow-always"
    );
    assert_eq!(pending_after_safe[0]["id"], risky_id);

    let risky_reply = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/permission/reply")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(format!(
                    r#"{{"id":"{risky_id}","decision":"allow"}}"#
                )))
                .expect("request"),
        )
        .await
        .expect("risky reply");
    assert_eq!(risky_reply.status(), StatusCode::OK);

    let safe_response = tokio::time::timeout(Duration::from_secs(3), safe_turn)
        .await
        .expect("safe turn completed")
        .expect("safe turn join");
    assert_eq!(safe_response.status(), StatusCode::OK);
    let risky_response = tokio::time::timeout(Duration::from_secs(3), risky_turn)
        .await
        .expect("risky turn completed")
        .expect("risky turn join");
    assert_eq!(risky_response.status(), StatusCode::OK);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn permission_reply_always_persists_and_reloads_after_restart_without_new_ask() {
    let scratch = Scratch::new("server-permission-always-persist-restart");
    let workspace = scratch.dir.join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace dir");
    std::fs::write(
        workspace.join("cortex.toml"),
        r#"
[[permission]]
tool = "bash"
pattern = "echo *"
action = "ask"

[[permission]]
tool = "bash"
pattern = "rm *"
action = "ask"
"#,
    )
    .expect("write project config");

    let app = server::app_with_state(
        server::ServerState::new(scratch.db_path())
            .with_cwd(workspace.clone())
            .with_config_env(Vec::new()),
    );
    let first_session = create_session(&app).await;
    let first_target = workspace.join("first.txt");
    let second_target = workspace.join("second.txt");
    let risky_target = workspace.join("risky-target.txt");

    let first_app = app.clone();
    let first_session_id = first_session.id.clone();
    let first_turn = tokio::spawn(async move {
        first_app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/session/{first_session_id}/command"))
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(r#"{"command":"echo first > first.txt"}"#))
                    .expect("request"),
            )
            .await
            .expect("first command response")
    });

    let mut pending = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline {
        let pending_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/permission/pending")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("pending response");
        assert_eq!(pending_response.status(), StatusCode::OK);
        let pending_body: serde_json::Value = json_body(pending_response).await;
        pending = pending_body["pending"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        if !pending.is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(
        pending.len(),
        1,
        "first command should be awaiting permission"
    );
    let first_id = pending[0]["id"].as_str().expect("pending id").to_owned();
    assert!(
        pending[0]["subject"]
            .as_str()
            .is_some_and(|subject| subject.contains("echo first")),
        "pending ask should match the echo command: {pending:?}"
    );

    let first_reply = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/permission/reply")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(format!(
                    r#"{{"id":"{first_id}","decision":"allow","always":true}}"#
                )))
                .expect("request"),
        )
        .await
        .expect("reply response");
    assert_eq!(first_reply.status(), StatusCode::OK);
    let first_reply_body: serde_json::Value = json_body(first_reply).await;
    assert_eq!(first_reply_body["resolved"], 1);

    let first_response = tokio::time::timeout(Duration::from_secs(3), first_turn)
        .await
        .expect("first command completed")
        .expect("first command join");
    assert_eq!(first_response.status(), StatusCode::OK);
    let first_body: serde_json::Value = json_body(first_response).await;
    assert_eq!(first_body["status"], "completed");
    assert!(
        first_target.exists(),
        "allowed command should create first.txt"
    );

    let workspace_key = checkpoints::workspace_key(&workspace);
    let store_path = permissions::store::store_path_with(
        &permissions::store::StoreInputs::from_process(),
        &workspace_key,
    )
    .expect("permission store path");
    assert!(
        store_path.exists(),
        "allow-always should persist a saved decision after command completion"
    );
    let stored: Vec<permissions::SavedDecision> = permissions::store::read(&store_path);
    assert!(
        stored.iter().any(|decision| {
            decision.tool == "bash"
                && decision.pattern == "echo *"
                && decision.decision == permissions::Decision::Allow
        }),
        "saved decisions should include the scoped echo allow entry: {stored:?}"
    );

    let event_bus = Arc::new(events::EventBus::new());
    let mut event_stream = event_bus.subscribe_stream();
    let restarted = server::app_with_state(
        server::ServerState::new(scratch.db_path())
            .with_cwd(workspace.clone())
            .with_config_env(Vec::new())
            .with_event_bus(event_bus),
    );
    let restarted_session = create_session(&restarted).await;

    let second_response = restarted
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/session/{}/command", restarted_session.id))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"command":"echo second > second.txt"}"#))
                .expect("request"),
        )
        .await
        .expect("second command response");
    assert_eq!(second_response.status(), StatusCode::OK);
    let second_body: serde_json::Value = json_body(second_response).await;
    assert_eq!(
        second_body["status"], "completed",
        "restarted server should auto-allow matching command from saved decision"
    );
    assert!(
        second_target.exists(),
        "matching command should run after restart without a new permission prompt"
    );

    let pending_after_second = restarted
        .clone()
        .oneshot(
            Request::builder()
                .uri("/permission/pending")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("pending response");
    assert_eq!(pending_after_second.status(), StatusCode::OK);
    let pending_after_second_body: serde_json::Value = json_body(pending_after_second).await;
    assert_eq!(
        pending_after_second_body["pending"]
            .as_array()
            .map_or(0, std::vec::Vec::len),
        0,
        "matching saved decision should leave no pending permission entries"
    );

    let mut saw_permission_asked = false;
    loop {
        match event_stream.try_recv() {
            Ok(event) => {
                if event.kind() == "permission.asked" {
                    saw_permission_asked = true;
                }
            }
            Err(tokio::sync::broadcast::error::TryRecvError::Empty) => break,
            Err(tokio::sync::broadcast::error::TryRecvError::Closed) => break,
            Err(tokio::sync::broadcast::error::TryRecvError::Lagged(_)) => continue,
        }
    }
    assert!(
        !saw_permission_asked,
        "matching saved decision should not emit permission.asked after restart"
    );

    let restarted_for_risky = restarted.clone();
    let restarted_session_id = restarted_session.id.clone();
    let risky_turn = tokio::spawn(async move {
        restarted_for_risky
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/session/{restarted_session_id}/command"))
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(r#"{"command":"rm risky-target.txt"}"#))
                    .expect("request"),
            )
            .await
            .expect("risky response")
    });

    let mut risky_pending = Vec::new();
    let risky_deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < risky_deadline {
        let pending_response = restarted
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/permission/pending")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("pending response");
        assert_eq!(pending_response.status(), StatusCode::OK);
        let pending_body: serde_json::Value = json_body(pending_response).await;
        risky_pending = pending_body["pending"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        if !risky_pending.is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(
        risky_pending.len(),
        1,
        "unrelated command should still require permission despite saved echo allow"
    );
    let risky_id = risky_pending[0]["id"]
        .as_str()
        .expect("risky id")
        .to_owned();
    assert!(
        risky_pending[0]["subject"]
            .as_str()
            .is_some_and(|subject| subject.contains("rm risky-target.txt")),
        "pending subject should show unrelated risky command: {risky_pending:?}"
    );

    let risky_reply = restarted
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/permission/reply")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(format!(
                    r#"{{"id":"{risky_id}","decision":"deny"}}"#
                )))
                .expect("request"),
        )
        .await
        .expect("risky reply");
    assert_eq!(risky_reply.status(), StatusCode::OK);

    let risky_response = tokio::time::timeout(Duration::from_secs(3), risky_turn)
        .await
        .expect("risky command completed")
        .expect("risky command join");
    assert_eq!(risky_response.status(), StatusCode::OK);
    let risky_body: serde_json::Value = json_body(risky_response).await;
    assert_eq!(risky_body["status"], "error");
    assert!(
        !risky_target.exists(),
        "denied risky command must not create side effects"
    );

    let _ = std::fs::remove_file(&store_path);
}

#[tokio::test]
async fn saved_allow_does_not_override_trust_or_readonly_hard_denies() {
    let scratch = Scratch::new("server-permission-hard-deny-precedence");

    let trusted_workspace = scratch.dir.join("trusted-workspace");
    std::fs::create_dir_all(&trusted_workspace).expect("trusted workspace");
    std::fs::write(
        trusted_workspace.join("cortex.toml"),
        r#"
trust = "untrusted"

[[permission]]
tool = "bash"
pattern = "echo *"
action = "ask"
"#,
    )
    .expect("write trust config");
    let trusted_workspace_key = checkpoints::workspace_key(&trusted_workspace);
    permissions::store::save_for(
        &trusted_workspace_key,
        &[permissions::SavedDecision::new(
            "bash",
            "echo *",
            permissions::Decision::Allow,
        )],
    )
    .expect("seed trust saved decision");
    let trusted_app = server::app_with_state(
        server::ServerState::new(scratch.db_path())
            .with_cwd(trusted_workspace.clone())
            .with_config_env(Vec::new()),
    );
    let trusted_session = create_session(&trusted_app).await;
    let trusted_target = trusted_workspace.join("trust.txt");
    let trusted_response = trusted_app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/session/{}/command", trusted_session.id))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"command":"echo trust > trust.txt"}"#))
                .expect("request"),
        )
        .await
        .expect("trust command response");
    assert_eq!(trusted_response.status(), StatusCode::OK);
    let trusted_body: serde_json::Value = json_body(trusted_response).await;
    assert_eq!(
        trusted_body["status"], "error",
        "trust hard deny must outrank saved allow"
    );
    assert!(
        !trusted_target.exists(),
        "trust hard deny should block command side effects"
    );
    let trusted_pending = trusted_app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/permission/pending")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("pending response");
    let trusted_pending_body: serde_json::Value = json_body(trusted_pending).await;
    assert_eq!(
        trusted_pending_body["pending"]
            .as_array()
            .map_or(0, std::vec::Vec::len),
        0,
        "trust hard deny should not create pending asks"
    );

    let plan_workspace = scratch.dir.join("plan-workspace");
    std::fs::create_dir_all(&plan_workspace).expect("plan workspace");
    std::fs::write(
        plan_workspace.join("cortex.toml"),
        r#"
agent = "plan"

[[permission]]
tool = "bash"
pattern = "echo *"
action = "ask"
"#,
    )
    .expect("write plan config");
    let plan_workspace_key = checkpoints::workspace_key(&plan_workspace);
    permissions::store::save_for(
        &plan_workspace_key,
        &[permissions::SavedDecision::new(
            "bash",
            "echo *",
            permissions::Decision::Allow,
        )],
    )
    .expect("seed plan saved decision");
    let plan_app = server::app_with_state(
        server::ServerState::new(scratch.db_path())
            .with_cwd(plan_workspace.clone())
            .with_config_env(Vec::new()),
    );
    let plan_session = create_session(&plan_app).await;
    let plan_target = plan_workspace.join("plan.txt");
    let plan_response = plan_app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/session/{}/command", plan_session.id))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"command":"echo plan > plan.txt"}"#))
                .expect("request"),
        )
        .await
        .expect("plan command response");
    assert_eq!(plan_response.status(), StatusCode::OK);
    let plan_body: serde_json::Value = json_body(plan_response).await;
    assert_eq!(
        plan_body["status"], "error",
        "read-only agent hard deny must outrank saved allow"
    );
    assert!(
        !plan_target.exists(),
        "read-only mode should block command side effects"
    );
    let plan_pending = plan_app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/permission/pending")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("pending response");
    let plan_pending_body: serde_json::Value = json_body(plan_pending).await;
    assert_eq!(
        plan_pending_body["pending"]
            .as_array()
            .map_or(0, std::vec::Vec::len),
        0,
        "read-only hard deny should not create pending asks"
    );

    if let Some(path) = permissions::store::store_path_with(
        &permissions::store::StoreInputs::from_process(),
        &trusted_workspace_key,
    ) {
        let _ = std::fs::remove_file(path);
    }
    if let Some(path) = permissions::store::store_path_with(
        &permissions::store::StoreInputs::from_process(),
        &plan_workspace_key,
    ) {
        let _ = std::fs::remove_file(path);
    }
}

#[tokio::test]
async fn overlapping_same_session_prompt_is_queued_and_persists_in_order() {
    let scratch = Scratch::new("server-message-same-session-queue");
    let provider = Arc::new(EchoProvider::with_delay(Duration::from_secs(1)));
    let app = server::app_with_provider(scratch.db_path(), provider);
    let session = create_session(&app).await;

    let first_prompt = "first-overlap-prompt";
    let queued_prompt = "second-overlap-queued";
    let post_app = app.clone();
    let post_uri = format!("/session/{}/message", session.id);
    let first = tokio::spawn(async move {
        post_app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(post_uri)
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(format!(r#"{{"text":"{first_prompt}"}}"#)))
                    .expect("request"),
            )
            .await
            .expect("first response")
    });

    tokio::time::sleep(Duration::from_millis(50)).await;
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/session/{}/message", session.id))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(format!(r#"{{"text":"{queued_prompt}"}}"#)))
                .expect("request"),
        )
        .await
        .expect("second response");
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value = json_body(response).await;
    assert_eq!(body["status"], "completed");
    assert_eq!(body["queued"], true);
    assert!(
        body["content"]
            .as_str()
            .is_some_and(|text| text.contains(queued_prompt)),
        "queued prompt should eventually run and complete: {body:?}"
    );

    let first_response = tokio::time::timeout(Duration::from_secs(3), first)
        .await
        .expect("first prompt completed")
        .expect("first prompt join");
    assert_eq!(first_response.status(), StatusCode::OK);

    let response = app
        .clone()
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
    assert_eq!(
        messages
            .iter()
            .filter(|message| message["role"] == "user")
            .count(),
        2,
        "both prompts should persist as queued admission: {body:?}"
    );
    assert!(
        messages
            .iter()
            .any(|message| message["content"] == first_prompt),
        "the accepted prompt should still persist: {body:?}"
    );
    assert!(
        messages
            .iter()
            .any(|message| message["content"] == queued_prompt),
        "the queued prompt should persist after running: {body:?}"
    );
    let user_contents: Vec<&str> = messages
        .iter()
        .filter(|message| message["role"] == "user")
        .filter_map(|message| message["content"].as_str())
        .collect();
    assert_eq!(
        user_contents,
        vec![first_prompt, queued_prompt],
        "queued prompt should be admitted after the active prompt in submission order"
    );
}

#[tokio::test]
async fn malformed_message_model_is_rejected_without_persisting_or_calling_provider() {
    let scratch = Scratch::new("server-message-invalid-model");
    let provider = Arc::new(EchoProvider::default());
    let app = server::app_with_provider(scratch.db_path(), provider.clone());
    let session = create_session(&app).await;

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/session/{}/message", session.id))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    r#"{"text":"this must not run","model":"not-a-real-model"}"#,
                ))
                .expect("request"),
        )
        .await
        .expect("message response");

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_json_content_type(&response);
    let body: serde_json::Value = json_body(response).await;
    assert!(
        body["error"].as_str().is_some_and(|message| {
            message.contains("not-a-real-model") && message.contains("provider/model")
        }),
        "invalid-model error should name the model and expected form: {body:?}"
    );
    assert_eq!(provider.calls(), 0, "invalid model must not call provider");

    let response = app
        .clone()
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
        "invalid model must be rejected before persisting a user message: {body:?}"
    );
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
        .clone()
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

    let resumed = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/session/{}/message", session.id))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"text":"resume after abort"}"#))
                .expect("request"),
        )
        .await
        .expect("resumed prompt response");
    assert_eq!(resumed.status(), StatusCode::OK);
    let resumed_body: serde_json::Value = json_body(resumed).await;
    assert_eq!(resumed_body["status"], "completed");
    assert_eq!(resumed_body["queued"], false);
    assert!(
        resumed_body["content"]
            .as_str()
            .is_some_and(|text| text.contains("resume after abort")),
        "session should remain resumable after abort: {resumed_body:?}"
    );
}

#[tokio::test]
async fn abort_without_active_remote_turn_does_not_leave_a_stale_remote_marker() {
    let scratch = Scratch::new("server-abort-idle-no-marker");
    let app = server::app_with_provider(scratch.db_path(), Arc::new(EchoProvider::default()));
    let session = create_session(&app).await;
    let marker = remote_abort_marker_path(&session.id);
    let _ = std::fs::remove_file(&marker);

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
    let body: serde_json::Value = json_body(response).await;
    assert_eq!(body["status"], "abort_requested");
    assert_eq!(
        body["active"], false,
        "idle abort should report no active turn"
    );
    assert!(
        !marker.exists(),
        "idle abort should not leave a stale remote marker that can poison later TUI turns"
    );
}

#[tokio::test]
async fn abort_during_tool_execution_cleans_up_and_keeps_session_resumable() {
    let scratch = Scratch::new("server-message-abort-tool");
    let workspace = scratch.dir.join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace dir");
    let marker = workspace.join("tool-orphan-marker.txt");
    let _ = std::fs::remove_file(&marker);

    let provider = Arc::new(AbortableToolProvider::new(marker.clone()));
    let state =
        server::ServerState::with_provider(scratch.db_path(), provider).with_cwd(&workspace);
    let app = server::app_with_state(state);
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
                    .body(Body::from(r#"{"text":"run a long tool command"}"#))
                    .expect("request"),
            )
            .await
            .expect("post response")
    });

    tokio::time::sleep(Duration::from_millis(80)).await;
    let abort = app
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
    assert_eq!(abort.status(), StatusCode::OK);
    let abort_body: serde_json::Value = json_body(abort).await;
    assert_eq!(abort_body["status"], "abort_requested");
    assert_eq!(abort_body["active"], true);

    let response = tokio::time::timeout(Duration::from_secs(3), post)
        .await
        .expect("prompt stopped promptly")
        .expect("prompt join");
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value = json_body(response).await;
    assert_eq!(body["status"], "aborted");
    assert_eq!(body["queued"], false);

    tokio::time::sleep(Duration::from_millis(2300)).await;
    assert!(
        !marker.exists(),
        "aborting a tool turn should clean up in-flight tool processes before they finish side effects"
    );

    let resumed = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/session/{}/message", session.id))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"text":"resume after tool abort"}"#))
                .expect("request"),
        )
        .await
        .expect("resumed prompt");
    assert_eq!(resumed.status(), StatusCode::OK);
    let resumed_body: serde_json::Value = json_body(resumed).await;
    assert_eq!(resumed_body["status"], "completed");
    assert_eq!(resumed_body["queued"], false);
    assert_eq!(resumed_body["role"], "assistant");
}

#[tokio::test]
async fn server_tool_edit_records_checkpoint_revertible_by_tui_store() {
    let scratch = Scratch::new("server-checkpoint-edit");
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

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/session/{}/message", session.id))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    r#"{"text":"edit target.txt from H0 to H1 using tools"}"#,
                ))
                .expect("request"),
        )
        .await
        .expect("message response");
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value = json_body(response).await;
    assert_eq!(body["status"], "completed");
    assert_eq!(
        std::fs::read_to_string(&target).expect("edited target"),
        "H1 server checkpoint target\n",
        "the server-driven tool edit should change the file"
    );

    let store = checkpoints::ShadowStore::open(
        &workspace,
        &checkpoint_base.join(checkpoints::workspace_key(&workspace)),
    )
    .expect("reopen checkpoint store like attached TUI");
    assert!(
        store.history().expect("history").can_undo(),
        "server-originated edit should append an undoable checkpoint"
    );
    let undo = store.undo().expect("undo through attached TUI store");
    assert!(
        matches!(undo, checkpoints::RestoreOutcome::Restored { .. }),
        "undo should restore a recorded turn, got {undo:?}"
    );
    assert_eq!(
        std::fs::read_to_string(&target).expect("restored target"),
        h0,
        "the attached TUI checkpoint store should restore the exact pre-edit bytes"
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
async fn todo_route_returns_tool_written_todos_after_app_recreation() {
    let scratch = Scratch::new("server-todo-durable");
    let provider = Arc::new(TodoWritingProvider::default());
    let first_app = server::app_with_provider(scratch.db_path(), provider.clone());
    let session = create_session(&first_app).await;

    let response = first_app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/session/{}/message", session.id))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"text":"record the validation todos"}"#))
                .expect("request"),
        )
        .await
        .expect("message response");
    assert_eq!(response.status(), StatusCode::OK);

    let relaunched_app = server::app_with_provider(scratch.db_path(), provider);
    for path in ["todo", "todos"] {
        let response = relaunched_app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/session/{}/{path}", session.id))
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("todo list response");
        assert_eq!(response.status(), StatusCode::OK);
        assert_json_content_type(&response);
        let body: serde_json::Value = json_body(response).await;
        assert_eq!(body["session"]["id"], session.id);
        assert_eq!(body["todos"].as_array().expect("todos").len(), 3);
        assert_eq!(body["todos"][0]["content"], "capture message nonce");
        assert_eq!(body["todos"][0]["status"], "completed");
        assert_eq!(body["todos"][1]["status"], "in_progress");
        assert_eq!(body["todos"][2]["status"], "pending");
    }
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

fn remote_abort_marker_path(session_id: &str) -> PathBuf {
    let base = std::env::var("CORTEX_DATA_DIR")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var("XDG_DATA_HOME")
                .ok()
                .filter(|value| !value.trim().is_empty())
                .map(|value| PathBuf::from(value).join("cortex"))
        })
        .or_else(|| {
            std::env::var("HOME")
                .ok()
                .filter(|value| !value.trim().is_empty())
                .map(|value| PathBuf::from(value).join(".local/share/cortex"))
        })
        .unwrap_or_else(|| std::env::temp_dir().join("cortex"));
    base.join("remote-abort")
        .join(format!("{session_id}.abort"))
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
    seen_models: Mutex<Vec<String>>,
    seen_systems: Mutex<Vec<String>>,
    delay: Option<Duration>,
}

#[derive(Debug, Default)]
struct CheckpointEditingProvider {
    calls: AtomicUsize,
}

#[derive(Debug, Default)]
struct TodoWritingProvider {
    calls: AtomicUsize,
}

#[derive(Debug, Default)]
struct BashRmProvider {
    calls: AtomicUsize,
}

#[derive(Debug, Default)]
struct PermissionPendingProvider {
    calls: AtomicUsize,
}

#[derive(Debug, Default)]
struct ScopedPermissionPendingProvider {
    calls: AtomicUsize,
}

#[derive(Debug, Default)]
struct RichPartsProvider {
    calls: AtomicUsize,
}

#[derive(Debug)]
struct AbortableToolProvider {
    calls: AtomicUsize,
    marker: PathBuf,
}

impl AbortableToolProvider {
    fn new(marker: PathBuf) -> Self {
        Self {
            calls: AtomicUsize::new(0),
            marker,
        }
    }
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

impl Provider for BashRmProvider {
    fn stream<'a>(&'a self, _request: LlmRequest) -> BoxFuture<'a, Result<LlmStream, LlmError>> {
        Box::pin(async move {
            let call = self.calls.fetch_add(1, Ordering::SeqCst);
            let events = match call {
                0 => vec![
                    LlmEvent::ToolCall(ToolCall {
                        id: "call_rm".to_owned(),
                        name: "bash".to_owned(),
                        arguments: r#"{"command":"rm victim.txt"}"#.to_owned(),
                    }),
                    LlmEvent::Done {
                        reason: FinishReason::ToolCalls,
                    },
                ],
                _ => vec![
                    LlmEvent::TextDelta {
                        text: "bash attempt finished".to_owned(),
                    },
                    LlmEvent::Usage(Usage {
                        input_tokens: 13,
                        output_tokens: 5,
                        total_tokens: 18,
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

impl Provider for PermissionPendingProvider {
    fn stream<'a>(&'a self, request: LlmRequest) -> BoxFuture<'a, Result<LlmStream, LlmError>> {
        Box::pin(async move {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let has_tool_result = request
                .messages
                .iter()
                .any(|message| message.role == Role::Tool);
            let events = if !has_tool_result {
                vec![
                    LlmEvent::ToolCall(ToolCall {
                        id: "call_permission".to_owned(),
                        name: "bash".to_owned(),
                        arguments: r#"{"command":"echo permission-check"}"#.to_owned(),
                    }),
                    LlmEvent::Done {
                        reason: FinishReason::ToolCalls,
                    },
                ]
            } else {
                vec![
                    LlmEvent::TextDelta {
                        text: "permission resolved".to_owned(),
                    },
                    LlmEvent::Usage(Usage {
                        input_tokens: 9,
                        output_tokens: 4,
                        total_tokens: 13,
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

impl Provider for ScopedPermissionPendingProvider {
    fn stream<'a>(&'a self, request: LlmRequest) -> BoxFuture<'a, Result<LlmStream, LlmError>> {
        Box::pin(async move {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let has_tool_result = request
                .messages
                .iter()
                .any(|message| message.role == Role::Tool);
            let events = if !has_tool_result {
                let prompt = request
                    .messages
                    .iter()
                    .rev()
                    .find(|message| message.role == Role::User)
                    .map(|message| message.content.as_str())
                    .unwrap_or_default();
                let command = if prompt.contains("risky") {
                    "rm risky-target.txt"
                } else {
                    "echo safe-check"
                };
                vec![
                    LlmEvent::ToolCall(ToolCall {
                        id: "call_permission_scope".to_owned(),
                        name: "bash".to_owned(),
                        arguments: serde_json::json!({ "command": command }).to_string(),
                    }),
                    LlmEvent::Done {
                        reason: FinishReason::ToolCalls,
                    },
                ]
            } else {
                vec![
                    LlmEvent::TextDelta {
                        text: "permission resolved".to_owned(),
                    },
                    LlmEvent::Usage(Usage {
                        input_tokens: 9,
                        output_tokens: 4,
                        total_tokens: 13,
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

impl Provider for TodoWritingProvider {
    fn stream<'a>(&'a self, _request: LlmRequest) -> BoxFuture<'a, Result<LlmStream, LlmError>> {
        Box::pin(async move {
            let call = self.calls.fetch_add(1, Ordering::SeqCst);
            let events = match call {
                0 => vec![
                    LlmEvent::ToolCall(ToolCall {
                        id: "call_todo".to_owned(),
                        name: "todo".to_owned(),
                        arguments: serde_json::json!({
                            "todos": [
                                {"content": "capture message nonce", "status": "completed"},
                                {"content": "verify TUI snapshot", "status": "in_progress"},
                                {"content": "verify server response", "status": "pending"}
                            ]
                        })
                        .to_string(),
                    }),
                    LlmEvent::Done {
                        reason: FinishReason::ToolCalls,
                    },
                ],
                _ => vec![
                    LlmEvent::TextDelta {
                        text: "todos recorded".to_owned(),
                    },
                    LlmEvent::Usage(Usage {
                        input_tokens: 13,
                        output_tokens: 5,
                        total_tokens: 18,
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

impl Provider for RichPartsProvider {
    fn stream<'a>(&'a self, request: LlmRequest) -> BoxFuture<'a, Result<LlmStream, LlmError>> {
        Box::pin(async move {
            let call = self.calls.fetch_add(1, Ordering::SeqCst);
            let has_tool_result = request
                .messages
                .iter()
                .any(|message| message.role == Role::Tool);
            let events = if call == 0 && !has_tool_result {
                vec![
                    LlmEvent::Reasoning {
                        text: "planning tool usage".to_owned(),
                    },
                    LlmEvent::ToolCall(ToolCall {
                        id: "call_rich_todo".to_owned(),
                        name: "todo".to_owned(),
                        arguments: serde_json::json!({
                            "todos": [
                                {"content": "rich part item", "status": "completed"}
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
                        text: "rich parts recorded".to_owned(),
                    },
                    LlmEvent::Usage(Usage {
                        input_tokens: 12,
                        output_tokens: 4,
                        total_tokens: 16,
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

impl Provider for AbortableToolProvider {
    fn stream<'a>(&'a self, request: LlmRequest) -> BoxFuture<'a, Result<LlmStream, LlmError>> {
        Box::pin(async move {
            let call = self.calls.fetch_add(1, Ordering::SeqCst);
            let has_tool_result = request
                .messages
                .iter()
                .any(|message| message.role == Role::Tool);
            let events = if call == 0 && !has_tool_result {
                let marker = self
                    .marker
                    .to_string_lossy()
                    .replace('\\', "\\\\")
                    .replace('"', "\\\"");
                let command = format!("sleep 2; printf orphan > \"{marker}\"");
                vec![
                    LlmEvent::ToolCall(ToolCall {
                        id: "call_abort_bash".to_owned(),
                        name: "bash".to_owned(),
                        arguments: serde_json::json!({ "command": command }).to_string(),
                    }),
                    LlmEvent::Done {
                        reason: FinishReason::ToolCalls,
                    },
                ]
            } else {
                vec![
                    LlmEvent::TextDelta {
                        text: "tool cleanup complete".to_owned(),
                    },
                    LlmEvent::Usage(Usage {
                        input_tokens: 10,
                        output_tokens: 5,
                        total_tokens: 15,
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

impl EchoProvider {
    fn with_delay(delay: Duration) -> Self {
        Self {
            calls: AtomicUsize::new(0),
            seen_models: Mutex::new(Vec::new()),
            seen_systems: Mutex::new(Vec::new()),
            delay: Some(delay),
        }
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }

    fn seen_models(&self) -> Vec<String> {
        self.seen_models.lock().expect("seen models lock").clone()
    }

    fn seen_systems(&self) -> Vec<String> {
        self.seen_systems.lock().expect("seen systems lock").clone()
    }
}

impl Provider for EchoProvider {
    fn stream<'a>(&'a self, request: LlmRequest) -> BoxFuture<'a, Result<LlmStream, LlmError>> {
        Box::pin(async move {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.seen_models
                .lock()
                .expect("seen models lock")
                .push(request.model.clone());
            if let Some(system) = request
                .messages
                .iter()
                .find(|message| message.role == Role::System)
            {
                self.seen_systems
                    .lock()
                    .expect("seen systems lock")
                    .push(system.content.clone());
            }
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
