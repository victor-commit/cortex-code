//! Black-box tests for the remote MCP client against a real remote MCP server fixture.
//!
//! The `cortex-mcp-remote-server` fixture is spawned as a child HTTP server (loopback, ports
//! 3200-3299) in one of three modes and the client is driven through the public `mcp` API:
//!
//! - Streamable HTTP connect + tools/list + a real tools/call (VAL-MCP-006).
//! - SSE fallback when the server rejects Streamable HTTP (VAL-MCP-007).
//! - A runtime tool-set change reflected without a restart + an `mcp.tools.changed` event
//!   (VAL-MCP-008).
//! - The OAuth authorization-code flow: `needs_auth` → connected via a CSRF-validated loopback
//!   callback, a forged state is rejected (VAL-MCP-009), and tokens are persisted to a 0600 store,
//!   reused without re-prompting, refreshed on expiry, and never leaked (VAL-MCP-010).
//! - Every spawned stdio MCP descendant is terminated by PID on shutdown (VAL-MCP-011).

use std::collections::BTreeMap;
use std::os::unix::fs::PermissionsExt;
use std::process::{Child, Command};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use events::{Event, EventBus};
use mcp::{
    ConnectionState, CredentialStore, McpConnectOptions, McpManager, McpServerConfig, oauth,
    servers_from_value,
};
use serde_json::json;

fn remote_server_bin() -> String {
    env!("CARGO_BIN_EXE_cortex-mcp-remote-server").to_owned()
}

fn echo_server_bin() -> String {
    env!("CARGO_BIN_EXE_cortex-mcp-echo-server").to_owned()
}

fn config(value: serde_json::Value) -> BTreeMap<String, McpServerConfig> {
    servers_from_value(&value).expect("valid mcp config")
}

fn pid_alive(pid: u32) -> bool {
    std::path::Path::new(&format!("/proc/{pid}")).exists()
}

async fn wait_until_gone(pid: u32) -> bool {
    for _ in 0..50 {
        if !pid_alive(pid) {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    false
}

fn temp_path(tag: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "cortex-mcp-remote-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ))
}

type ChangeEvents = Arc<Mutex<Vec<(String, Vec<String>)>>>;
type ResourceEvents = Arc<Mutex<Vec<(String, Vec<String>)>>>;
type PromptEvents = Arc<Mutex<Vec<(String, Vec<String>)>>>;
type LogEvents = Arc<Mutex<Vec<serde_json::Value>>>;

/// A spawned fixture child, killed (with its descendants by PID) when the guard drops.
struct Fixture {
    child: Child,
}

impl Fixture {
    fn spawn(mode: &str, port: u16, log: Option<&std::path::Path>, extra: &[(&str, &str)]) -> Self {
        let mut command = Command::new(remote_server_bin());
        command
            .env("CORTEX_MCP_MODE", mode)
            .env("CORTEX_MCP_PORT", port.to_string());
        if let Some(log) = log {
            command.env("CORTEX_MCP_LOG", log);
        }
        for (key, value) in extra {
            command.env(key, value);
        }
        let child = command.spawn().expect("spawn the remote MCP fixture");
        Self { child }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Wait until `port` accepts a TCP connection (the fixture's listener is bound).
async fn wait_for_port(port: u16) {
    for _ in 0..200 {
        if tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .is_ok()
        {
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("the fixture never started listening on 127.0.0.1:{port}");
}

/// Pick a free loopback port in the MCP fixture band (3200-3299).
/// Hardcoded fixtures collide with other local services (e.g. uvicorn on 3210).
fn free_mcp_port() -> u16 {
    use std::net::TcpListener;
    for port in 3200..=3299 {
        if TcpListener::bind(("127.0.0.1", port)).is_ok() {
            return port;
        }
    }
    panic!("no free MCP fixture port in 3200-3299");
}

#[tokio::test]
async fn remote_streamable_connects_lists_and_calls() {
    let log = temp_path("streamable-log");
    let port = free_mcp_port();
    let _fixture = Fixture::spawn("streamable", port, Some(&log), &[]);
    wait_for_port(port).await;

    let servers = config(json!({
        "api": { "type": "remote", "url": format!("http://127.0.0.1:{port}/mcp") }
    }));
    let manager = McpManager::connect(servers).await;

    // VAL-MCP-006: the remote server connects over HTTP, completes the handshake, and lists tools.
    let status = &manager.statuses()[0];
    assert_eq!(
        status.state,
        ConnectionState::Connected,
        "remote streamable connect failed: {:?}",
        status.error
    );
    assert!(
        status.pid.is_none(),
        "a remote server has no spawned child PID"
    );

    let mut ids: Vec<String> = manager.tools().iter().map(|t| t.id.clone()).collect();
    ids.sort();
    assert_eq!(ids, vec!["api_add", "api_echo", "api_search"]);

    // A real tools/call reaches the server and returns its genuine result.
    let result = manager
        .call("api_add", json!({ "a": 2, "b": 40 }))
        .await
        .expect("add call succeeds");
    assert!(!result.is_error);
    assert!(result.content.contains("42"), "got: {}", result.content);

    manager.shutdown().await;

    let log_contents = std::fs::read_to_string(&log).unwrap_or_default();
    assert!(
        log_contents.contains("HTTP MCP session established (streamable)"),
        "server log must confirm an HTTP MCP session, got:\n{log_contents}"
    );
    let _ = std::fs::remove_file(&log);
}

#[tokio::test]
async fn remote_streamable_connect_preserves_configured_authorization_header() {
    let log = temp_path("streamable-auth-log");
    let _fixture = Fixture::spawn(
        "streamable",
        3219,
        Some(&log),
        &[("CORTEX_MCP_REQUIRE_AUTHORIZATION", "Bearer static-token")],
    );
    wait_for_port(3219).await;

    let servers = config(json!({
        "api": {
            "type": "remote",
            "url": "http://127.0.0.1:3219/mcp",
            "headers": { "authorization": "Bearer static-token" }
        }
    }));
    let manager = McpManager::connect(servers).await;

    assert_eq!(
        manager.statuses()[0].state,
        ConnectionState::Connected,
        "the configured Authorization header must be sent during the real handshake: {:?}",
        manager.statuses()[0].error
    );
    assert!(manager.has_tool("api_add"));
    manager.shutdown().await;
    let _ = std::fs::remove_file(&log);
}

#[tokio::test]
async fn remote_falls_back_to_sse_when_streamable_rejected() {
    let log = temp_path("sse-log");
    let _fixture = Fixture::spawn("sse", 3211, Some(&log), &[]);
    wait_for_port(3211).await;

    let servers = config(json!({
        "api": { "type": "remote", "url": "http://127.0.0.1:3211/sse" }
    }));
    let manager = McpManager::connect(servers).await;

    // VAL-MCP-007: an SSE-only server (rejects Streamable HTTP) still connects via the SSE fallback.
    let status = &manager.statuses()[0];
    assert_eq!(
        status.state,
        ConnectionState::Connected,
        "SSE fallback connect failed: {:?}",
        status.error
    );
    assert!(manager.has_tool("api_search"), "tools listed over SSE");

    let result = manager
        .call("api_echo", json!({ "message": "hi-sse" }))
        .await
        .expect("echo over sse");
    assert!(result.content.contains("hi-sse"), "got: {}", result.content);

    manager.shutdown().await;

    let log_contents = std::fs::read_to_string(&log).unwrap_or_default();
    assert!(
        log_contents.contains("SSE endpoint opened"),
        "server log must show the SSE endpoint, got:\n{log_contents}"
    );
    let _ = std::fs::remove_file(&log);
}

#[tokio::test]
async fn runtime_tool_change_is_reflected_and_emits_event() {
    let bus = Arc::new(EventBus::new());
    let changed: ChangeEvents = Arc::new(Mutex::new(Vec::new()));
    let sink = changed.clone();
    bus.subscribe(Arc::new(move |event: &Event| {
        if let Event::McpToolsChanged(payload) = event {
            sink.lock()
                .unwrap()
                .push((payload.server.clone(), payload.tools.clone()));
        }
    }));

    let _fixture = Fixture::spawn("streamable", 3212, None, &[]);
    wait_for_port(3212).await;

    let servers = config(json!({
        "api": { "type": "remote", "url": "http://127.0.0.1:3212/mcp" }
    }));
    let manager = McpManager::connect_with(
        servers,
        McpConnectOptions {
            bus: Some(bus.clone()),
            credentials: None,
            roots: Vec::new(),
            reserved_tool_ids: Vec::new(),
        },
    )
    .await;
    assert_eq!(manager.statuses()[0].state, ConnectionState::Connected);
    assert!(
        !manager.has_tool("api_extra"),
        "extra tool absent initially"
    );

    // Trigger a runtime tool-set change on the live server (no cortex restart).
    reqwest::Client::new()
        .post("http://127.0.0.1:3212/control/add-tool")
        .send()
        .await
        .expect("control add-tool");

    // VAL-MCP-008: the change is reflected without a restart.
    let mut reflected = false;
    for _ in 0..200 {
        if manager.has_tool("api_extra") {
            reflected = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert!(
        reflected,
        "the added tool must appear without restarting cortex"
    );

    // ... and an mcp.tools.changed event was observed naming the new tool.
    let mut emitted = false;
    for _ in 0..40 {
        if changed
            .lock()
            .unwrap()
            .iter()
            .any(|(server, tools)| server == "api" && tools.iter().any(|t| t == "api_extra"))
        {
            emitted = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert!(emitted, "an mcp.tools.changed event must name the new tool");

    // A removal is reflected too.
    reqwest::Client::new()
        .post("http://127.0.0.1:3212/control/remove-tool")
        .send()
        .await
        .expect("control remove-tool");
    let mut removed = false;
    for _ in 0..200 {
        if !manager.has_tool("api_extra") {
            removed = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert!(
        removed,
        "the removed tool must disappear without restarting cortex"
    );

    manager.shutdown().await;
}

#[tokio::test]
async fn resource_and_prompt_changes_are_reflected_and_emit_events() {
    let bus = Arc::new(EventBus::new());
    let resources: ResourceEvents = Arc::new(Mutex::new(Vec::new()));
    let prompts: PromptEvents = Arc::new(Mutex::new(Vec::new()));
    let resources_sink = resources.clone();
    let prompts_sink = prompts.clone();
    bus.subscribe(Arc::new(move |event: &Event| match event {
        Event::McpResourcesChanged(payload) => {
            resources_sink
                .lock()
                .unwrap()
                .push((payload.server.clone(), payload.resources.clone()));
        }
        Event::McpPromptsChanged(payload) => {
            prompts_sink
                .lock()
                .unwrap()
                .push((payload.server.clone(), payload.prompts.clone()));
        }
        _ => {}
    }));

    let _fixture = Fixture::spawn("streamable", 3220, None, &[]);
    wait_for_port(3220).await;
    let servers = config(json!({
        "api": { "type": "remote", "url": "http://127.0.0.1:3220/mcp" }
    }));
    let manager = McpManager::connect_with(
        servers,
        McpConnectOptions {
            bus: Some(bus),
            credentials: None,
            roots: Vec::new(),
            reserved_tool_ids: Vec::new(),
        },
    )
    .await;
    assert_eq!(manager.statuses()[0].state, ConnectionState::Connected);
    assert!(
        !manager.has_resource("notes://demo"),
        "fixture starts without dynamic resource"
    );
    assert!(
        !manager.has_prompt("greet"),
        "fixture starts without dynamic prompt"
    );

    reqwest::Client::new()
        .post("http://127.0.0.1:3220/control/add-resource")
        .send()
        .await
        .expect("control add-resource");
    reqwest::Client::new()
        .post("http://127.0.0.1:3220/control/add-prompt")
        .send()
        .await
        .expect("control add-prompt");

    // VAL-GAP-MCP-003: resources/prompts list-changed notifications refresh the live catalogue.
    let mut reflected = false;
    for _ in 0..200 {
        if manager.has_resource("notes://demo") && manager.has_prompt("greet") {
            reflected = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert!(
        reflected,
        "resource and prompt catalogue updates must be visible without reconnect"
    );

    let mut emitted = false;
    for _ in 0..80 {
        let resources_changed = resources.lock().unwrap().iter().any(|(server, entries)| {
            server == "api" && entries.iter().any(|uri| uri == "notes://demo")
        });
        let prompts_changed =
            prompts.lock().unwrap().iter().any(|(server, entries)| {
                server == "api" && entries.iter().any(|name| name == "greet")
            });
        if resources_changed && prompts_changed {
            emitted = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert!(
        emitted,
        "resource and prompt list changes must emit typed events"
    );

    manager.shutdown().await;
}

#[tokio::test]
async fn live_capability_changes_keep_routes_unique_and_never_retarget_ids() {
    let log = temp_path("live-collision-log");
    let _fixture = Fixture::spawn("streamable", 3222, Some(&log), &[]);
    wait_for_port(3222).await;
    let servers = config(json!({
        "api": { "type": "remote", "url": "http://127.0.0.1:3222/mcp" }
    }));
    let manager = McpManager::connect_with(
        servers,
        McpConnectOptions {
            bus: None,
            credentials: None,
            roots: Vec::new(),
            reserved_tool_ids: vec!["apply_patch".to_owned()],
        },
    )
    .await;
    assert_eq!(manager.statuses()[0].state, ConnectionState::Connected);

    for endpoint in [
        "/control/add-collision-tools",
        "/control/add-resource",
        "/control/add-prompt",
    ] {
        reqwest::Client::new()
            .post(format!("http://127.0.0.1:3222{endpoint}"))
            .send()
            .await
            .expect("control update");
    }

    let mut assigned = None;
    for _ in 0..200 {
        let tool = manager
            .tools()
            .into_iter()
            .find(|tool| tool.tool == "resource_notes://demo");
        let resource = manager
            .resources()
            .into_iter()
            .find(|resource| resource.uri == "notes://demo");
        let prompt = manager
            .prompts()
            .into_iter()
            .find(|prompt| prompt.name == "greet");
        if let (Some(tool), Some(resource), Some(prompt)) = (tool, resource, prompt) {
            assigned = Some((tool.id, resource.id, prompt.id));
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    let (tool_id, resource_id, prompt_id) =
        assigned.expect("all live collision capabilities should be reflected");
    assert_ne!(tool_id, resource_id);
    assert_ne!(
        prompt_id,
        manager
            .tools()
            .into_iter()
            .find(|tool| tool.tool == "prompt_greet")
            .expect("prompt collision tool")
            .id
    );

    let mut ids = manager
        .capabilities()
        .into_iter()
        .map(|capability| capability.id)
        .collect::<Vec<_>>();
    let total = ids.len();
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), total, "live catalogue contains no duplicate ids");

    assert!(
        manager
            .call(&tool_id, json!({}))
            .await
            .expect("typed tool route")
            .content
            .contains("live collision tool resource_notes://demo")
    );
    assert!(
        manager
            .read_resource(&resource_id)
            .await
            .expect("typed resource route")
            .content
            .contains("REMOTE-RESOURCE-CONTENT")
    );
    assert!(
        manager
            .get_prompt(&prompt_id, json!({ "name": "live" }))
            .await
            .expect("typed prompt route")
            .content
            .contains("Hello live from remote MCP")
    );

    reqwest::Client::new()
        .post("http://127.0.0.1:3222/control/remove-collision-tools")
        .send()
        .await
        .expect("remove collision tools");
    for _ in 0..200 {
        if !manager
            .tools()
            .iter()
            .any(|tool| tool.tool == "resource_notes://demo")
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert!(
        !manager
            .tools()
            .iter()
            .any(|tool| tool.tool == "resource_notes://demo"),
        "removed capability leaves the live catalogue"
    );

    reqwest::Client::new()
        .post("http://127.0.0.1:3222/control/add-collision-tools")
        .send()
        .await
        .expect("re-add collision tools");
    let mut readded = None;
    for _ in 0..200 {
        if let Some(tool) = manager
            .tools()
            .into_iter()
            .find(|tool| tool.tool == "resource_notes://demo")
        {
            readded = Some(tool.id);
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    let readded = readded.expect("re-added capability should be reflected");
    assert_eq!(
        readded, tool_id,
        "a live remove/re-add cannot silently retarget an existing model-visible id"
    );

    manager.shutdown().await;
    let log_contents = std::fs::read_to_string(&log).expect("fixture log");
    for expected in [
        "tools/call name=resource_notes://demo",
        "resources/read uri=notes://demo",
        "prompts/get name=greet",
    ] {
        assert!(
            log_contents.contains(expected),
            "fixture log missing `{expected}`:\n{log_contents}"
        );
    }
    let _ = std::fs::remove_file(log);
}

#[tokio::test]
async fn logging_notifications_are_bridged_redacted_and_non_disruptive() {
    let bus = Arc::new(EventBus::new());
    let logs: LogEvents = Arc::new(Mutex::new(Vec::new()));
    let sink = logs.clone();
    bus.subscribe(Arc::new(move |event: &Event| {
        if let Event::McpLogMessage(payload) = event {
            sink.lock()
                .unwrap()
                .push(serde_json::to_value(payload).expect("serialize mcp.log payload"));
        }
    }));

    let _fixture = Fixture::spawn("streamable", 3221, None, &[]);
    wait_for_port(3221).await;
    let servers = config(json!({
        "api": { "type": "remote", "url": "http://127.0.0.1:3221/mcp" }
    }));
    let manager = McpManager::connect_with(
        servers,
        McpConnectOptions {
            bus: Some(bus),
            credentials: None,
            roots: Vec::new(),
            reserved_tool_ids: Vec::new(),
        },
    )
    .await;
    assert_eq!(manager.statuses()[0].state, ConnectionState::Connected);

    reqwest::Client::new()
        .post("http://127.0.0.1:3221/control/log-message")
        .send()
        .await
        .expect("control log-message");

    let mut observed = None;
    for _ in 0..80 {
        if let Some(payload) = logs.lock().unwrap().last().cloned() {
            observed = Some(payload);
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    let payload = observed.expect("mcp.log event should be emitted");
    let serialized = serde_json::to_string(&payload).expect("serialize log payload");
    assert_eq!(payload["server"], "api");
    assert_eq!(payload["level"], "warning");
    assert_eq!(payload["logger"], "fixture.remote");
    assert!(
        serialized.contains(redaction::PLACEHOLDER),
        "log payload should carry redacted data: {serialized}"
    );
    assert!(
        !serialized.contains("deadbeef"),
        "redacted payload must not leak secret-shaped content: {serialized}"
    );

    // VAL-GAP-MCP-002: notifications/message should not interrupt normal tool usage.
    let result = manager
        .call("api_echo", json!({ "message": "still-running" }))
        .await
        .expect("tool call after logging notification");
    assert!(
        result.content.contains("still-running"),
        "manager must remain usable after a log notification, got: {}",
        result.content
    );

    manager.shutdown().await;
}

#[tokio::test]
async fn oauth_flow_connects_persists_reuses_and_never_leaks() {
    let log = temp_path("oauth-log");
    let store_path = temp_path("oauth-store").join("credentials.json");
    let credentials = Arc::new(CredentialStore::at(store_path));
    let _fixture = Fixture::spawn("oauth", 3213, Some(&log), &[]);
    wait_for_port(3213).await;

    let url = "http://127.0.0.1:3213/mcp";
    let servers = config(json!({ "api": { "type": "remote", "url": url } }));

    // Without a token the OAuth-gated server reports needs_auth (not failed).
    let manager = McpManager::connect_with(
        servers.clone(),
        McpConnectOptions {
            bus: None,
            credentials: Some(credentials.clone()),
            roots: Vec::new(),
            reserved_tool_ids: Vec::new(),
        },
    )
    .await;
    assert_eq!(
        manager.statuses()[0].state,
        ConnectionState::NeedsAuth,
        "an OAuth-gated server must surface needs_auth: {:?}",
        manager.statuses()[0].error
    );

    // VAL-MCP-009: complete the authorization-code flow through a CSRF-validated loopback callback.
    let oauth_config = oauth::discover(url, "cortex").await.expect("discover");
    let request = oauth::begin(oauth_config, Some(3214))
        .await
        .expect("begin auth");
    let authorize_url = request.authorize_url.clone();
    let completion = tokio::spawn(async move { request.complete(Duration::from_secs(10)).await });

    // Drive the "browser": visiting the authorize URL redirects to the loopback callback.
    let response = reqwest::Client::builder()
        .build()
        .unwrap()
        .get(&authorize_url)
        .send()
        .await
        .expect("authorize request");
    assert!(response.status().is_success(), "callback should answer 200");

    let token = completion
        .await
        .expect("join")
        .expect("token exchange succeeds");
    credentials.put("api", token).expect("persist token");

    // VAL-MCP-010: the store is mode 0600.
    let mode = std::fs::metadata(credentials.path())
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(mode & 0o777, 0o600, "the credential store must be 0600");

    // Reconnect reusing the stored token, without re-prompting, and the server is connected.
    let reconnected = McpManager::connect_with(
        servers.clone(),
        McpConnectOptions {
            bus: None,
            credentials: Some(credentials.clone()),
            roots: Vec::new(),
            reserved_tool_ids: Vec::new(),
        },
    )
    .await;
    assert_eq!(
        reconnected.statuses()[0].state,
        ConnectionState::Connected,
        "the stored token must be reused to connect: {:?}",
        reconnected.statuses()[0].error
    );
    assert!(reconnected.has_tool("api_add"));
    let result = reconnected
        .call("api_add", json!({ "a": 1, "b": 1 }))
        .await
        .expect("authenticated call");
    assert!(result.content.contains('2'));
    reconnected.shutdown().await;

    // The access token never appears in the server's logs.
    let access = credentials.get("api").unwrap().unwrap().access_token;
    let log_contents = std::fs::read_to_string(&log).unwrap_or_default();
    assert!(
        !log_contents.contains(&access),
        "the access token must never be written to logs"
    );
    let stdout = String::new();
    let stderr = String::new();
    assert!(
        !stdout.contains(&access) && !stderr.contains(&access),
        "the access token must never be printed to stdout/stderr"
    );

    let _ = std::fs::remove_file(&log);
    let _ = std::fs::remove_dir_all(credentials.path().parent().unwrap());
}

#[tokio::test]
async fn oauth_401_is_typed_needs_auth_without_exposing_the_challenge() {
    let challenge = "Bearer realm=\"fixture\", opaque_challenge=\"challenge-must-not-leak\"";
    let _fixture = Fixture::spawn(
        "oauth",
        3240,
        None,
        &[("CORTEX_MCP_WWW_AUTHENTICATE", challenge)],
    );
    wait_for_port(3240).await;
    let servers = config(json!({
        "api": { "type": "remote", "url": "http://127.0.0.1:3240/mcp" }
    }));

    let manager = McpManager::connect(servers).await;
    let status = &manager.statuses()[0];
    assert_eq!(status.state, ConnectionState::NeedsAuth);
    let rendered = format!("{status:?}");
    assert!(
        !rendered.contains(challenge) && !rendered.contains("challenge-must-not-leak"),
        "a server OAuth challenge must not escape the typed status projection: {rendered}"
    );
    assert!(
        status
            .public_error()
            .as_deref()
            .is_some_and(|error| error.contains("authorization required")),
        "needs_auth must retain an actionable safe message: {:?}",
        status.public_error()
    );
    manager.shutdown().await;
}

#[tokio::test]
async fn oauth_rejects_a_forged_callback_state() {
    let _fixture = Fixture::spawn("oauth", 3215, None, &[]);
    wait_for_port(3215).await;

    let oauth_config = oauth::discover("http://127.0.0.1:3215/mcp", "cortex")
        .await
        .expect("discover");
    let rejected = oauth::begin(oauth_config.clone(), Some(3199)).await;
    assert!(
        matches!(rejected, Err(oauth::OAuthError::InvalidCallbackPort(3199))),
        "callback ports outside 3200-3299 must be refused"
    );
    let request = oauth::begin(oauth_config, None).await.expect("begin auth");
    let redirect_uri = request.redirect_uri.clone();
    let callback_port = url::Url::parse(&redirect_uri)
        .expect("redirect URI")
        .port()
        .expect("callback port");
    assert!(
        (3200..=3299).contains(&callback_port),
        "callback port must stay in the mission range, got {callback_port}"
    );
    let completion = tokio::spawn(async move { request.complete(Duration::from_secs(5)).await });

    // VAL-MCP-009: a forged/mismatched state is rejected.
    let forged = format!("{redirect_uri}?code=attacker-code&state=not-the-issued-state");
    let _ = reqwest::Client::new().get(&forged).send().await;

    let outcome = completion.await.expect("join");
    assert!(
        matches!(outcome, Err(oauth::OAuthError::StateMismatch)),
        "a forged callback state must be rejected, got: {outcome:?}"
    );

    // Callback listeners must be released after error paths too.
    let rebound = tokio::net::TcpListener::bind(("127.0.0.1", callback_port)).await;
    assert!(
        rebound.is_ok(),
        "callback port should be free after state mismatch, bind failed: {rebound:?}"
    );
}

#[tokio::test]
async fn oauth_token_is_refreshed_on_expiry() {
    let log = temp_path("oauth-refresh-log");
    let store_path = temp_path("oauth-refresh-store").join("credentials.json");
    let credentials = Arc::new(CredentialStore::at(store_path));
    // Issue short-lived (1s) access tokens so the next connect must refresh.
    let _fixture = Fixture::spawn("oauth", 3217, Some(&log), &[("CORTEX_MCP_EXPIRES_IN", "1")]);
    wait_for_port(3217).await;

    let url = "http://127.0.0.1:3217/mcp";
    let servers = config(json!({ "api": { "type": "remote", "url": url } }));

    let oauth_config = oauth::discover(url, "cortex").await.expect("discover");
    let request = oauth::begin(oauth_config, Some(3218))
        .await
        .expect("begin auth");
    let authorize_url = request.authorize_url.clone();
    let completion = tokio::spawn(async move { request.complete(Duration::from_secs(10)).await });
    reqwest::Client::new()
        .get(&authorize_url)
        .send()
        .await
        .expect("authorize");
    let token = completion.await.expect("join").expect("token");
    let first_access = token.access_token.clone();
    assert!(token.refresh_token.is_some(), "a refresh token is issued");
    credentials.put("api", token).expect("persist");

    // Let the access token expire.
    tokio::time::sleep(Duration::from_millis(1500)).await;

    // VAL-MCP-010: the expired token is refreshed in place on the next connect.
    let manager = McpManager::connect_with(
        servers,
        McpConnectOptions {
            bus: None,
            credentials: Some(credentials.clone()),
            roots: Vec::new(),
            reserved_tool_ids: Vec::new(),
        },
    )
    .await;
    assert_eq!(
        manager.statuses()[0].state,
        ConnectionState::Connected,
        "the refreshed token must connect: {:?}",
        manager.statuses()[0].error
    );
    manager.shutdown().await;

    let stored = credentials.get("api").unwrap().unwrap();
    assert_ne!(
        stored.access_token, first_access,
        "the stored access token must be replaced by the refreshed one"
    );

    let log_contents = std::fs::read_to_string(&log).unwrap_or_default();
    assert!(
        log_contents.contains("refresh_token grant"),
        "the server must have served a refresh_token grant, got:\n{log_contents}"
    );

    let _ = std::fs::remove_file(&log);
    let _ = std::fs::remove_dir_all(credentials.path().parent().unwrap());
}

#[tokio::test]
async fn oauth_auto_callback_port_is_in_range_and_released_after_completion() {
    let _fixture = Fixture::spawn("oauth", 3223, None, &[]);
    wait_for_port(3223).await;

    let oauth_config = oauth::discover("http://127.0.0.1:3223/mcp", "cortex")
        .await
        .expect("discover");
    let request = oauth::begin(oauth_config, None).await.expect("begin auth");
    let redirect_url = url::Url::parse(&request.redirect_uri).expect("redirect URI");
    let callback_port = redirect_url.port().expect("callback port");
    assert!(
        (3200..=3299).contains(&callback_port),
        "callback port must stay in the mission range, got {callback_port}"
    );

    let authorize_url = request.authorize_url.clone();
    let completion = tokio::spawn(async move { request.complete(Duration::from_secs(10)).await });
    let response = reqwest::Client::new()
        .get(&authorize_url)
        .send()
        .await
        .expect("authorize request");
    assert!(response.status().is_success());
    completion
        .await
        .expect("join auth completion")
        .expect("oauth completion succeeds");

    // VAL-GAP-MCP-004: callback listener is released once the flow completes.
    let rebound = tokio::net::TcpListener::bind(("127.0.0.1", callback_port)).await;
    assert!(
        rebound.is_ok(),
        "callback port should be free after completion, bind failed: {rebound:?}"
    );
}

#[tokio::test]
async fn stdio_descendants_are_killed_by_pid_on_shutdown() {
    let bin = echo_server_bin();
    let pid_file = temp_path("grandchild-pid");
    let servers = config(json!({
        "demo": {
            "type": "local",
            "command": [bin],
            "environment": {
                "CORTEX_MCP_ECHO_SPAWN_CHILD": "1",
                "CORTEX_MCP_ECHO_CHILD_PID_FILE": pid_file.to_string_lossy()
            }
        }
    }));

    let manager = McpManager::connect(servers).await;
    let child_pid = manager.statuses()[0]
        .pid
        .expect("a live PID for the connected server");
    assert!(pid_alive(child_pid), "the direct child must be running");

    // Read the grandchild PID the server forked.
    let mut grandchild_pid = None;
    for _ in 0..100 {
        if let Ok(contents) = std::fs::read_to_string(&pid_file) {
            if let Ok(pid) = contents.trim().parse::<u32>() {
                grandchild_pid = Some(pid);
                break;
            }
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    let grandchild_pid = grandchild_pid.expect("the server must report its forked grandchild PID");
    assert!(pid_alive(grandchild_pid), "the grandchild must be running");

    // VAL-MCP-011: shutdown terminates the whole tree by PID; no orphan survives.
    manager.shutdown().await;

    let mut both_gone = false;
    for _ in 0..50 {
        if !pid_alive(child_pid) && !pid_alive(grandchild_pid) {
            both_gone = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(
        both_gone,
        "the MCP server child ({child_pid}) and grandchild ({grandchild_pid}) must both be terminated"
    );
    let _ = std::fs::remove_file(&pid_file);
}

#[tokio::test]
async fn cancelled_stdio_setup_reaps_spawned_server_and_descendant() {
    let bin = echo_server_bin();
    let pid_file = temp_path("cancelled-setup-grandchild-pid");
    let servers = config(json!({
        "demo": {
            "type": "local",
            "command": [bin],
            "environment": {
                "CORTEX_MCP_ECHO_SPAWN_CHILD": "1",
                "CORTEX_MCP_ECHO_CHILD_PID_FILE": pid_file.to_string_lossy(),
                "CORTEX_MCP_ECHO_STALL_BEFORE_SERVE": "1"
            }
        }
    }));

    // `connect` is deliberately left waiting in initialize after the fixture has spawned its
    // descendant. Cancelling this future simulates a process lifecycle signal arriving during MCP
    // setup, before a manager exists to perform normal shutdown.
    let task = tokio::spawn(async move { McpManager::connect(servers).await });
    let mut grandchild_pid = None;
    for _ in 0..100 {
        if let Ok(contents) = std::fs::read_to_string(&pid_file)
            && let Ok(pid) = contents.trim().parse::<u32>()
        {
            grandchild_pid = Some(pid);
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    task.abort();
    let _ = task.await;

    let grandchild_pid = grandchild_pid.expect("the stalled server must report its descendant PID");
    let mut reaped = false;
    for _ in 0..50 {
        if !pid_alive(grandchild_pid) {
            reaped = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(
        reaped,
        "cancelling setup must reap the spawned descendant ({grandchild_pid})"
    );
    let _ = std::fs::remove_file(&pid_file);
}

#[tokio::test]
async fn cancelled_manager_setup_reaps_previously_connected_and_inflight_trees() {
    let bin = echo_server_bin();
    let first_log = temp_path("cancelled-manager-first-log");
    let first_descendant_file = temp_path("cancelled-manager-first-descendant");
    let inflight_log = temp_path("cancelled-manager-inflight-log");
    let inflight_descendant_file = temp_path("cancelled-manager-inflight-descendant");
    let servers = config(json!({
        "a-connected": {
            "type": "local",
            "command": [bin.clone()],
            "environment": {
                "CORTEX_MCP_ECHO_LOG": first_log.to_string_lossy(),
                "CORTEX_MCP_ECHO_SPAWN_CHILD": "1",
                "CORTEX_MCP_ECHO_CHILD_PID_FILE": first_descendant_file.to_string_lossy()
            }
        },
        "b-inflight": {
            "type": "local",
            "command": [bin],
            "environment": {
                "CORTEX_MCP_ECHO_LOG": inflight_log.to_string_lossy(),
                "CORTEX_MCP_ECHO_SPAWN_CHILD": "1",
                "CORTEX_MCP_ECHO_CHILD_PID_FILE": inflight_descendant_file.to_string_lossy(),
                "CORTEX_MCP_ECHO_STALL_BEFORE_SERVE": "1"
            }
        }
    }));

    // The first server has completed setup and is only owned by the manager-local collection when
    // the second server stalls. Aborting here must reap both exact trees, not just the in-flight
    // connection protected by McpClient's own spawn guard.
    let task = tokio::spawn(async move { McpManager::connect(servers).await });
    let inflight_descendant = wait_for_pid_file(&inflight_descendant_file).await;
    let first_descendant = wait_for_pid_file(&first_descendant_file).await;
    let first_pid = started_pid_from_log(&first_log).expect("first server wrote its PID");
    let inflight_pid = started_pid_from_log(&inflight_log).expect("in-flight server wrote its PID");
    task.abort();
    let _ = task.await;

    for pid in [
        first_pid,
        first_descendant,
        inflight_pid,
        inflight_descendant,
    ] {
        assert!(
            wait_until_gone(pid).await,
            "cancelling manager setup must reap exact PID {pid}"
        );
    }
    let _ = std::fs::remove_file(first_log);
    let _ = std::fs::remove_file(first_descendant_file);
    let _ = std::fs::remove_file(inflight_log);
    let _ = std::fs::remove_file(inflight_descendant_file);
}

async fn wait_for_pid_file(path: &std::path::Path) -> u32 {
    for _ in 0..100 {
        if let Ok(contents) = std::fs::read_to_string(path)
            && let Ok(pid) = contents.trim().parse::<u32>()
        {
            return pid;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("fixture did not report a PID in {}", path.display());
}

fn started_pid_from_log(path: &std::path::Path) -> Option<u32> {
    let contents = std::fs::read_to_string(path).ok()?;
    contents.lines().find_map(|line| {
        line.split_once("pid=")
            .and_then(|(_, tail)| tail.split_whitespace().next())
            .and_then(|pid| pid.parse::<u32>().ok())
    })
}
