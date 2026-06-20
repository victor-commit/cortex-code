//! Black-box regression test for the stdio MCP client: spawn the real `cortex-mcp-echo-server`
//! child, complete the handshake, list its tools, call them for real, and prove the child is killed
//! by PID on shutdown with no orphan.

use std::collections::BTreeMap;
use std::path::Path;
use std::time::{Duration, Instant};

use mcp::{McpManager, McpServerConfig, servers_from_value};
use serde_json::json;

/// The path to the compiled echo-server fixture binary (Cargo sets this for the package's tests).
fn echo_server_bin() -> String {
    env!("CARGO_BIN_EXE_cortex-mcp-echo-server").to_owned()
}

fn config(value: serde_json::Value) -> BTreeMap<String, McpServerConfig> {
    servers_from_value(&value).expect("valid mcp config")
}

/// Whether a PID is still present in the process table (Linux `/proc`).
fn pid_alive(pid: u32) -> bool {
    std::path::Path::new(&format!("/proc/{pid}")).exists()
}

fn temp_path(tag: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "cortex-mcp-stdio-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ))
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

fn started_pid_from_log(path: &Path) -> Option<u32> {
    let contents = std::fs::read_to_string(path).ok()?;
    contents.lines().find_map(|line| {
        line.split_once("pid=")
            .and_then(|(_, tail)| tail.split_whitespace().next())
            .and_then(|pid| pid.parse::<u32>().ok())
    })
}

#[tokio::test]
async fn connects_lists_calls_and_kills_child_on_shutdown() {
    let bin = echo_server_bin();
    let servers = config(json!({
        "demo": { "type": "local", "command": [bin] }
    }));

    let manager = McpManager::connect(servers).await;

    // VAL-MCP-001: the server is spawned and reported connected with a live PID.
    let statuses = manager.statuses();
    assert_eq!(statuses.len(), 1);
    let demo = &statuses[0];
    assert_eq!(demo.name, "demo");
    assert_eq!(demo.state, mcp::ConnectionState::Connected);
    let pid = demo.pid.expect("a live PID for the connected server");
    assert!(pid_alive(pid), "the spawned server child must be running");

    // VAL-MCP-002: tools/list surfaces exactly the server's advertised tools, server-qualified.
    let mut ids: Vec<String> = manager.tools().iter().map(|t| t.id.clone()).collect();
    ids.sort();
    assert_eq!(
        ids,
        vec![
            "demo_add",
            "demo_big",
            "demo_echo",
            "demo_env",
            "demo_search"
        ]
    );
    // Schemas are relayed (no fabrication): the `add` tool advertises its a/b integer params.
    let tools = manager.tools();
    let add = tools.iter().find(|t| t.id == "demo_add").unwrap();
    assert_eq!(add.input_schema["properties"]["a"]["type"], "integer");
    assert_eq!(add.tool, "add");

    // VAL-MCP-004: a real tools/call returns the server's genuine result.
    let result = manager
        .call("demo_add", json!({ "a": 21, "b": 21 }))
        .await
        .expect("add call succeeds");
    assert!(!result.is_error);
    assert!(result.content.contains("42"), "got: {}", result.content);
    assert!(
        result.content.contains("\"sum\":42"),
        "structured relayed: {}",
        result.content
    );

    // VAL-MCP-005: oversized output is truncated with a spill notice rather than silently lost.
    let big = manager
        .call("demo_big", json!({ "size": 200_000 }))
        .await
        .expect("big call succeeds");
    assert!(
        big.content.contains("output truncated"),
        "got head: {:.80}",
        big.content
    );
    assert!(big.content.len() < 200_000, "must be truncated");

    // The shutdown kills the child by PID — no orphan survives.
    manager.shutdown().await;
    let mut gone = false;
    for _ in 0..50 {
        if !pid_alive(pid) {
            gone = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert!(
        gone,
        "the MCP server child (pid {pid}) must be terminated on shutdown"
    );
}

#[tokio::test]
async fn two_servers_exporting_search_get_distinct_callable_ids() {
    let bin = echo_server_bin();
    let servers = config(json!({
        "a": { "type": "local", "command": [bin], "environment": { "CORTEX_MCP_ECHO_LABEL": "a" } },
        "b": { "type": "local", "command": [bin], "environment": { "CORTEX_MCP_ECHO_LABEL": "b" } }
    }));

    let manager = McpManager::connect(servers).await;

    // VAL-MCP-003: each server's `search` is exposed under a distinct, server-qualified id.
    assert!(
        manager.has_tool("a_search"),
        "a_search present: {:?}",
        tool_ids(&manager)
    );
    assert!(
        manager.has_tool("b_search"),
        "b_search present: {:?}",
        tool_ids(&manager)
    );

    // Both are individually callable and route to their own server (distinct labels prove it).
    let from_a = manager
        .call("a_search", json!({ "query": "x" }))
        .await
        .expect("a_search");
    let from_b = manager
        .call("b_search", json!({ "query": "x" }))
        .await
        .expect("b_search");
    assert!(
        from_a.content.contains("a: results for x"),
        "got: {}",
        from_a.content
    );
    assert!(
        from_b.content.contains("b: results for x"),
        "got: {}",
        from_b.content
    );

    manager.shutdown().await;
}

#[tokio::test]
async fn server_log_records_the_real_call_arguments() {
    let bin = echo_server_bin();
    let log = std::env::temp_dir().join(format!("cortex-mcp-echo-log-{}.txt", std::process::id()));
    let _ = std::fs::remove_file(&log);

    let servers = config(json!({
        "demo": {
            "type": "local",
            "command": [bin],
            "environment": { "CORTEX_MCP_ECHO_LOG": log.to_string_lossy() }
        }
    }));

    let manager = McpManager::connect(servers).await;
    let _ = manager
        .call("demo_add", json!({ "a": 21, "b": 21 }))
        .await
        .expect("add");
    manager.shutdown().await;

    let contents = std::fs::read_to_string(&log).expect("server log written");
    assert!(
        contents.contains("name=add")
            && contents.contains("\"a\":21")
            && contents.contains("\"b\":21"),
        "server log should record the real call args, got:\n{contents}"
    );
    let _ = std::fs::remove_file(&log);
}

#[tokio::test]
async fn a_bad_command_is_reported_failed_without_panicking() {
    let servers = config(json!({
        "broken": { "type": "local", "command": ["/nonexistent/mcp-bin-xyz"] }
    }));
    let manager = McpManager::connect(servers).await;
    let status = &manager.statuses()[0];
    assert_eq!(status.state, mcp::ConnectionState::Failed);
    assert!(status.error.is_some());
    assert!(manager.tools().is_empty());
    manager.shutdown().await;
}

#[tokio::test]
async fn stdio_server_observes_configured_cwd_and_env_without_host_secret_leak() {
    // VAL-MCP-015: a stdio server declared with a specific cwd and environment is launched with
    // exactly that working directory and those env vars, and the host OPENROUTER_API_KEY does not
    // leak into the child. The child inherits the test process environment (which, under live
    // validation, carries the real key) minus the stripped host secrets, so querying the key in the
    // child reports it absent.
    let bin = echo_server_bin();
    let work = std::env::temp_dir().join(format!("cortex-mcp-cwd-{}", std::process::id()));
    std::fs::create_dir_all(&work).expect("create work dir");
    let canonical_work = std::fs::canonicalize(&work).expect("canonicalize work dir");

    let servers = config(json!({
        "demo": {
            "type": "local",
            "command": [bin],
            "cwd": canonical_work.to_string_lossy(),
            "environment": { "FOO": "bar" }
        }
    }));

    let manager = McpManager::connect(servers).await;

    // The child reports the configured cwd and the configured FOO env var.
    let report = manager
        .call("demo_env", json!({ "name": "FOO" }))
        .await
        .expect("env call succeeds");
    assert!(
        report
            .content
            .contains(&canonical_work.to_string_lossy().to_string()),
        "the child must report the configured cwd, got: {}",
        report.content
    );
    assert!(
        report.content.contains("FOO=bar"),
        "the child must observe the configured FOO env var, got: {}",
        report.content
    );

    // The host OPENROUTER_API_KEY is stripped from the child: querying it reports it unset, and the
    // parent's value (present under live validation) never appears in the child's environment.
    let key_report = manager
        .call("demo_env", json!({ "name": "OPENROUTER_API_KEY" }))
        .await
        .expect("env call succeeds");
    assert!(
        key_report.content.contains("OPENROUTER_API_KEY=(unset)"),
        "the host OpenRouter key must be stripped from the child, got: {}",
        key_report.content
    );
    if let Ok(parent_key) = std::env::var("OPENROUTER_API_KEY") {
        if !parent_key.is_empty() {
            assert!(
                !key_report.content.contains(&parent_key),
                "the host OpenRouter key value must never leak into the child's environment"
            );
        }
    }

    manager.shutdown().await;
    let _ = std::fs::remove_dir_all(&work);
}

#[tokio::test]
async fn a_disabled_server_is_not_spawned() {
    let bin = echo_server_bin();
    let servers = config(json!({
        "demo": { "type": "local", "command": [bin], "enabled": false }
    }));
    let manager = McpManager::connect(servers).await;
    assert_eq!(manager.statuses()[0].state, mcp::ConnectionState::Disabled);
    assert!(manager.statuses()[0].pid.is_none());
    assert!(manager.tools().is_empty());
}

#[tokio::test]
async fn resources_and_prompts_are_listed_read_and_fetched() {
    let bin = echo_server_bin();
    let log = temp_path("resources-prompts-log");
    let nonce = format!("RESOURCE-CONTENT-{}", std::process::id());
    let servers = config(json!({
        "demo": {
            "type": "local",
            "command": [bin],
            "environment": {
                "CORTEX_MCP_ECHO_LOG": log.to_string_lossy(),
                "CORTEX_MCP_ECHO_ENABLE_RESOURCES_PROMPTS": "1",
                "CORTEX_MCP_ECHO_RESOURCE_CONTENT": nonce
            }
        }
    }));

    let manager = McpManager::connect(servers).await;
    let status = &manager.statuses()[0];
    assert_eq!(status.state, mcp::ConnectionState::Connected);

    // VAL-MCP-016: prompts/resources are enumerated alongside tools with server-advertised ids.
    assert!(manager.has_tool("demo_add"));
    let resources = manager.resources();
    assert_eq!(resources.len(), 1, "resources: {resources:?}");
    assert_eq!(resources[0].uri, "notes://demo");
    assert_eq!(status.resources[0].uri, "notes://demo");
    let prompts = manager.prompts();
    assert_eq!(prompts.len(), 1, "prompts: {prompts:?}");
    assert_eq!(prompts[0].name, "greet");
    assert_eq!(status.prompts[0].name, "greet");

    // VAL-MCP-020: resources/read retrieves the real server content, not just the listing.
    let read = manager
        .read_resource(&resources[0].id)
        .await
        .expect("resource read succeeds");
    assert!(read.content.contains(&nonce), "got: {}", read.content);

    // VAL-MCP-021: prompts/get applies arguments and returns the server's real prompt messages.
    let prompt = manager
        .get_prompt(&prompts[0].id, json!({ "name": "cortex" }))
        .await
        .expect("prompt get succeeds");
    assert!(
        prompt.content.contains("Hello cortex from MCP"),
        "got: {}",
        prompt.content
    );

    manager.shutdown().await;
    let log_contents = std::fs::read_to_string(&log).expect("server log written");
    assert!(
        log_contents.contains("resources/read uri=notes://demo"),
        "server log must confirm a real resources/read, got:\n{log_contents}"
    );
    assert!(
        log_contents.contains("prompts/get name=greet")
            && log_contents.contains("\"name\":\"cortex\""),
        "server log must confirm a real prompts/get with args, got:\n{log_contents}"
    );
    let _ = std::fs::remove_file(&log);
}

#[tokio::test]
async fn mcp_tool_is_error_result_is_relayed_as_error() {
    let bin = echo_server_bin();
    let servers = config(json!({
        "demo": {
            "type": "local",
            "command": [bin],
            "environment": { "CORTEX_MCP_ECHO_ENABLE_ERROR_TOOL": "1" }
        }
    }));

    let manager = McpManager::connect(servers).await;
    assert!(
        manager.has_tool("demo_boom"),
        "tools: {:?}",
        tool_ids(&manager)
    );
    let result = manager
        .call("demo_boom", json!({}))
        .await
        .expect("boom returns an MCP tool result, not a protocol error");
    assert!(result.is_error, "isError must propagate");
    assert!(
        result.content.contains("MCP-TOOL-FAILED"),
        "server error message must be preserved, got: {}",
        result.content
    );
    manager.shutdown().await;
}

#[tokio::test]
async fn stdio_initialize_timeout_is_bounded_and_kills_child() {
    let bin = echo_server_bin();
    let log = temp_path("stall-init-log");
    let servers = config(json!({
        "stall": {
            "type": "local",
            "command": [bin],
            "timeout": 1,
            "environment": {
                "CORTEX_MCP_ECHO_LOG": log.to_string_lossy(),
                "CORTEX_MCP_ECHO_STALL_BEFORE_SERVE": "1"
            }
        }
    }));

    let start = Instant::now();
    let manager = McpManager::connect(servers).await;
    assert!(
        start.elapsed() < Duration::from_secs(5),
        "connect must be bounded"
    );
    let status = &manager.statuses()[0];
    assert_eq!(status.state, mcp::ConnectionState::Failed);
    let error = status.error.as_deref().unwrap_or_default();
    assert!(
        error.contains("initialize") && error.contains("timed out"),
        "got: {error}"
    );

    let pid = started_pid_from_log(&log).expect("server wrote its PID before stalling");
    assert!(
        wait_until_gone(pid).await,
        "the stalled server child (pid {pid}) must be killed after initialize timeout"
    );
    manager.shutdown().await;
    let _ = std::fs::remove_file(&log);
}

#[tokio::test]
async fn stdio_server_exiting_immediately_is_reported_failed() {
    let bin = echo_server_bin();
    let servers = config(json!({
        "exits": {
            "type": "local",
            "command": [bin],
            "timeout": 2,
            "environment": { "CORTEX_MCP_ECHO_EXIT_IMMEDIATELY": "1" }
        }
    }));

    let start = Instant::now();
    let manager = McpManager::connect(servers).await;
    assert!(
        start.elapsed() < Duration::from_secs(5),
        "connect must be bounded"
    );
    let status = &manager.statuses()[0];
    assert_eq!(status.state, mcp::ConnectionState::Failed);
    assert!(status.error.is_some(), "a named failure reason is reported");
    assert!(manager.tools().is_empty());
    manager.shutdown().await;
}

#[tokio::test]
async fn stdio_tool_call_timeout_is_bounded_and_recoverable() {
    let bin = echo_server_bin();
    let servers = config(json!({
        "demo": {
            "type": "local",
            "command": [bin],
            "timeout": 1,
            "environment": { "CORTEX_MCP_ECHO_ENABLE_STALL_TOOL": "1" }
        }
    }));

    let manager = McpManager::connect(servers).await;
    let pid = manager.statuses()[0].pid.expect("live server pid");
    assert!(
        manager.has_tool("demo_stall"),
        "tools: {:?}",
        tool_ids(&manager)
    );

    let start = Instant::now();
    let err = manager
        .call("demo_stall", json!({}))
        .await
        .expect_err("stall must time out");
    assert!(
        start.elapsed() < Duration::from_secs(5),
        "call must be bounded"
    );
    let message = err.to_string();
    assert!(
        message.contains("tools/call") && message.contains("timed out"),
        "got: {message}"
    );

    manager.shutdown().await;
    assert!(
        wait_until_gone(pid).await,
        "the timed-out server child must be killed on shutdown"
    );
}

#[tokio::test]
async fn killed_stdio_server_next_call_returns_clear_error_without_panic() {
    let bin = echo_server_bin();
    let servers = config(json!({
        "demo": { "type": "local", "command": [bin], "timeout": 1 }
    }));

    let manager = McpManager::connect(servers).await;
    let pid = manager.statuses()[0].pid.expect("live server pid");
    assert!(pid_alive(pid));

    let status = tokio::process::Command::new("kill")
        .arg("-9")
        .arg(pid.to_string())
        .status()
        .await
        .expect("kill server by exact PID");
    assert!(status.success(), "kill command failed: {status}");
    assert!(wait_until_gone(pid).await, "server pid {pid} must be gone");

    let err = manager
        .call("demo_echo", json!({ "message": "after death" }))
        .await
        .expect_err("calling a dead server must fail cleanly");
    let message = err.to_string();
    assert!(
        message.contains("MCP server `demo`")
            && (message.contains("tools/call") || message.contains("not connected")),
        "got: {message}"
    );
    manager.shutdown().await;
}

fn tool_ids(manager: &McpManager) -> Vec<String> {
    manager.tools().iter().map(|t| t.id.clone()).collect()
}
