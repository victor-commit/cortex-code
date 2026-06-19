//! Black-box regression test for the stdio MCP client: spawn the real `cortex-mcp-echo-server`
//! child, complete the handshake, list its tools, call them for real, and prove the child is killed
//! by PID on shutdown with no orphan.

use std::collections::BTreeMap;

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
        vec!["demo_add", "demo_big", "demo_echo", "demo_search"]
    );
    // Schemas are relayed (no fabrication): the `add` tool advertises its a/b integer params.
    let add = manager.tools().iter().find(|t| t.id == "demo_add").unwrap();
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

fn tool_ids(manager: &McpManager) -> Vec<String> {
    manager.tools().iter().map(|t| t.id.clone()).collect()
}
