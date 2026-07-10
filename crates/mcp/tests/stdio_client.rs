//! Black-box regression test for the stdio MCP client: spawn the real `cortex-mcp-echo-server`
//! child, complete the handshake, list its tools, call them for real, and prove the child is killed
//! by PID on shutdown with no orphan.

use std::collections::BTreeMap;
use std::path::Path;
use std::time::{Duration, Instant};

use mcp::{CapabilityRoute, McpConnectOptions, McpManager, McpServerConfig, servers_from_value};
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

async fn descendant_pid_from_file(path: &Path) -> u32 {
    for _ in 0..100 {
        if let Ok(contents) = std::fs::read_to_string(path)
            && let Ok(pid) = contents.trim().parse::<u32>()
        {
            return pid;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!(
        "the fixture did not write its descendant PID to {}",
        path.display()
    );
}

async fn assert_tree_reaped(label: &str, pids: &[u32]) {
    for &pid in pids {
        assert!(
            wait_until_gone(pid).await,
            "{label} must reap PID {pid} by exact PID"
        );
    }
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
async fn stdio_child_receives_only_safe_baseline_and_explicit_environment() {
    // A subprocess gives this test a truly parent-inherited innocuously named secret without
    // mutating the shared integration-test process environment.
    const PROBE: &str = "CORTEX_MCP_CHILD_ENV_ALLOWLIST_PROBE";
    if std::env::var_os(PROBE).is_none() {
        let status =
            std::process::Command::new(std::env::current_exe().expect("resolve test binary"))
                .arg("stdio_child_receives_only_safe_baseline_and_explicit_environment")
                .arg("--exact")
                .arg("--nocapture")
                .env(PROBE, "1")
                .env(
                    "CORTEX_MCP_INNOCUOUS_PARENT_SECRET",
                    "parent-value-must-not-cross-the-mcp-boundary",
                )
                .env("XDG_CONFIG_HOME", "/tmp/cortex-parent-xdg-config")
                .status()
                .expect("run environment-isolation probe");
        assert!(status.success(), "environment-isolation child test failed");
        return;
    }

    let bin = echo_server_bin();
    let servers = config(json!({
        "demo": {
            "type": "local",
            "command": [bin],
            "environment": { "CORTEX_MCP_EXPLICIT_VALUE": "configured-value" }
        }
    }));
    let manager = McpManager::connect(servers).await;
    assert_eq!(manager.statuses()[0].state, mcp::ConnectionState::Connected);

    for forbidden in [
        "CORTEX_MCP_INNOCUOUS_PARENT_SECRET",
        "HOME",
        "XDG_CONFIG_HOME",
    ] {
        let report = manager
            .call("demo_env", json!({ "name": forbidden }))
            .await
            .expect("query child environment");
        assert!(
            report.content.contains(&format!("{forbidden}=(unset)")),
            "{forbidden} must not cross the stdio environment boundary: {}",
            report.content
        );
    }
    let configured = manager
        .call("demo_env", json!({ "name": "CORTEX_MCP_EXPLICIT_VALUE" }))
        .await
        .expect("query configured child environment");
    assert!(
        configured
            .content
            .contains("CORTEX_MCP_EXPLICIT_VALUE=configured-value"),
        "explicit per-server environment values must be forwarded: {}",
        configured.content
    );
    let path = manager
        .call("demo_env", json!({ "name": "PATH" }))
        .await
        .expect("query safe baseline PATH");
    assert!(
        !path.content.contains("PATH=(unset)"),
        "the documented execution baseline must preserve PATH: {}",
        path.content
    );

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
async fn optional_no_args_prompt_can_be_fetched_without_arguments() {
    let bin = echo_server_bin();
    let log = temp_path("prompt-noargs-log");
    let servers = config(json!({
        "demo": {
            "type": "local",
            "command": [bin],
            "environment": {
                "CORTEX_MCP_ECHO_LOG": log.to_string_lossy(),
                "CORTEX_MCP_ECHO_ENABLE_RESOURCES_PROMPTS": "1",
                "CORTEX_MCP_ECHO_ENABLE_NOARGS_PROMPT": "1"
            }
        }
    }));

    let manager = McpManager::connect(servers).await;
    assert_eq!(manager.statuses()[0].state, mcp::ConnectionState::Connected);
    let prompt = manager
        .prompts()
        .into_iter()
        .find(|entry| entry.name == "ping")
        .expect("fixture exposes a no-argument prompt");
    assert!(
        prompt.arguments.is_empty(),
        "no-argument fixture prompt should not declare required args"
    );

    let response = manager
        .get_prompt(&prompt.id, json!({}))
        .await
        .expect("no-argument prompt get succeeds");
    assert!(
        response.content.contains("Ping from MCP prompt fixture"),
        "got: {}",
        response.content
    );

    manager.shutdown().await;
    let log_contents = std::fs::read_to_string(&log).expect("server log written");
    assert!(
        log_contents.contains("prompts/get name=ping"),
        "server log must confirm a real prompts/get for the no-argument prompt, got:\n{log_contents}"
    );
    let _ = std::fs::remove_file(&log);
}

#[tokio::test]
async fn roots_capability_is_advertised_and_used_for_real_file_resolution() {
    let bin = echo_server_bin();
    let root = temp_path("roots");
    std::fs::create_dir_all(&root).expect("create roots dir");
    let relative = "root-note.txt";
    let content = format!("ROOT-CONTENT-{}", std::process::id());
    std::fs::write(root.join(relative), &content).expect("write fixture file");

    let servers = config(json!({
        "demo": {
            "type": "local",
            "command": [bin],
            "environment": { "CORTEX_MCP_ECHO_ENABLE_ROOTS_TOOL": "1" }
        }
    }));

    let manager = McpManager::connect_with(
        servers,
        McpConnectOptions {
            bus: None,
            credentials: None,
            roots: vec![root.clone()],
            reserved_tool_ids: Vec::new(),
        },
    )
    .await;
    assert_eq!(manager.statuses()[0].state, mcp::ConnectionState::Connected);
    assert!(
        manager.has_tool("demo_roots_read"),
        "tools: {:?}",
        tool_ids(&manager)
    );

    // VAL-GAP-MCP-001: the fixture requests roots/list from the client, confirms roots capability,
    // and uses the returned root URI to read a real file from disk.
    let result = manager
        .call("demo_roots_read", json!({ "path": relative }))
        .await
        .expect("roots_read call succeeds");
    assert!(
        !result.is_error,
        "roots_read returned an error: {}",
        result.content
    );
    assert!(
        result.content.contains(&content),
        "server must read file content from an advertised root, got: {}",
        result.content
    );
    assert!(
        result.content.contains("\"roots_capability\":true"),
        "server must observe roots capability in initialize params, got: {}",
        result.content
    );

    manager.shutdown().await;
    let _ = std::fs::remove_dir_all(root);
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
    let pid_file = temp_path("stall-init-grandchild-pid");
    let servers = config(json!({
        "stall": {
            "type": "local",
            "command": [bin],
            "timeout": 1,
            "environment": {
                "CORTEX_MCP_ECHO_LOG": log.to_string_lossy(),
                "CORTEX_MCP_ECHO_STALL_BEFORE_SERVE": "1",
                "CORTEX_MCP_ECHO_SPAWN_CHILD": "1",
                "CORTEX_MCP_ECHO_CHILD_PID_FILE": pid_file.to_string_lossy()
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
    let descendant = descendant_pid_from_file(&pid_file).await;
    assert_tree_reaped("initialize timeout", &[pid, descendant]).await;
    manager.shutdown().await;
    let _ = std::fs::remove_file(&log);
    let _ = std::fs::remove_file(&pid_file);
}

#[tokio::test]
async fn stdio_initialize_rejection_reaps_child_and_descendant() {
    let bin = echo_server_bin();
    let log = temp_path("reject-init-log");
    let pid_file = temp_path("reject-init-grandchild-pid");
    let servers = config(json!({
        "reject": {
            "type": "local",
            "command": [bin],
            "timeout": 1,
            "environment": {
                "CORTEX_MCP_ECHO_LOG": log.to_string_lossy(),
                "CORTEX_MCP_ECHO_REJECT_INITIALIZE": "1",
                "CORTEX_MCP_ECHO_SPAWN_CHILD": "1",
                "CORTEX_MCP_ECHO_CHILD_PID_FILE": pid_file.to_string_lossy()
            }
        }
    }));

    let manager = McpManager::connect(servers).await;
    let status = &manager.statuses()[0];
    assert_eq!(status.state, mcp::ConnectionState::Failed);
    assert!(
        status
            .error
            .as_deref()
            .is_some_and(|error| error.contains("initialize")),
        "expected a named initialize rejection: {:?}",
        status.error
    );

    let pid = started_pid_from_log(&log).expect("rejected server wrote its PID");
    let descendant = descendant_pid_from_file(&pid_file).await;
    assert_tree_reaped("initialize rejection", &[pid, descendant]).await;
    manager.shutdown().await;
    let _ = std::fs::remove_file(&log);
    let _ = std::fs::remove_file(&pid_file);
}

#[tokio::test]
async fn stdio_listing_rejection_reaps_child_and_descendant() {
    let bin = echo_server_bin();
    let log = temp_path("reject-list-log");
    let pid_file = temp_path("reject-list-grandchild-pid");
    let servers = config(json!({
        "reject": {
            "type": "local",
            "command": [bin],
            "timeout": 1,
            "environment": {
                "CORTEX_MCP_ECHO_LOG": log.to_string_lossy(),
                "CORTEX_MCP_ECHO_REJECT_LIST_TOOLS": "1",
                "CORTEX_MCP_ECHO_SPAWN_CHILD": "1",
                "CORTEX_MCP_ECHO_CHILD_PID_FILE": pid_file.to_string_lossy()
            }
        }
    }));

    let manager = McpManager::connect(servers).await;
    let status = &manager.statuses()[0];
    assert_eq!(status.state, mcp::ConnectionState::Failed);
    assert!(
        status
            .error
            .as_deref()
            .is_some_and(|error| error.contains("tools/list")),
        "expected a named listing rejection: {:?}",
        status.error
    );

    let pid = started_pid_from_log(&log).expect("listing-rejected server wrote its PID");
    let descendant = descendant_pid_from_file(&pid_file).await;
    assert_tree_reaped("tools/list rejection", &[pid, descendant]).await;
    manager.shutdown().await;
    let _ = std::fs::remove_file(&log);
    let _ = std::fs::remove_file(&pid_file);
}

#[tokio::test]
async fn stdio_listing_timeout_reaps_child_and_descendant() {
    let bin = echo_server_bin();
    let log = temp_path("stall-list-log");
    let pid_file = temp_path("stall-list-grandchild-pid");
    let servers = config(json!({
        "stall": {
            "type": "local",
            "command": [bin],
            "timeout": 1,
            "environment": {
                "CORTEX_MCP_ECHO_LOG": log.to_string_lossy(),
                "CORTEX_MCP_ECHO_STALL_LIST_TOOLS": "1",
                "CORTEX_MCP_ECHO_SPAWN_CHILD": "1",
                "CORTEX_MCP_ECHO_CHILD_PID_FILE": pid_file.to_string_lossy()
            }
        }
    }));

    let manager = McpManager::connect(servers).await;
    let status = &manager.statuses()[0];
    assert_eq!(status.state, mcp::ConnectionState::Failed);
    assert!(
        status
            .error
            .as_deref()
            .is_some_and(|error| error.contains("tools/list") && error.contains("timed out")),
        "expected a bounded listing timeout: {:?}",
        status.error
    );

    let pid = started_pid_from_log(&log).expect("listing-stalled server wrote its PID");
    let descendant = descendant_pid_from_file(&pid_file).await;
    assert_tree_reaped("tools/list timeout", &[pid, descendant]).await;
    manager.shutdown().await;
    let _ = std::fs::remove_file(&log);
    let _ = std::fs::remove_file(&pid_file);
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

#[tokio::test]
async fn one_global_namespace_routes_builtin_tools_resources_prompts_and_sanitized_servers() {
    let bin = echo_server_bin();
    let log = temp_path("global-capability-routing-log");
    let servers = config(json!({
        "apply": {
            "type": "local",
            "command": [bin.clone()],
            "environment": {
                "CORTEX_MCP_ECHO_LOG": log.to_string_lossy(),
                "CORTEX_MCP_ECHO_LABEL": "apply",
                "CORTEX_MCP_ECHO_EXTRA_TOOL": "patch"
            }
        },
        "plugin": {
            "type": "local",
            "command": [bin.clone()],
            "environment": {
                "CORTEX_MCP_ECHO_LOG": log.to_string_lossy(),
                "CORTEX_MCP_ECHO_LABEL": "plugin",
                "CORTEX_MCP_ECHO_EXTRA_TOOL": "tool"
            }
        },
        "demo": {
            "type": "local",
            "command": [bin.clone()],
            "environment": {
                "CORTEX_MCP_ECHO_LOG": log.to_string_lossy(),
                "CORTEX_MCP_ECHO_LABEL": "demo",
                "CORTEX_MCP_ECHO_ENABLE_RESOURCES_PROMPTS": "1",
                "CORTEX_MCP_ECHO_ENABLE_COLLISION_TOOLS": "1"
            }
        },
        "a_b": {
            "type": "local",
            "command": [bin.clone()],
            "environment": {
                "CORTEX_MCP_ECHO_LOG": log.to_string_lossy(),
                "CORTEX_MCP_ECHO_LABEL": "a_b",
                "CORTEX_MCP_ECHO_EXTRA_TOOL": "c"
            }
        },
        "a": {
            "type": "local",
            "command": [bin],
            "environment": {
                "CORTEX_MCP_ECHO_LOG": log.to_string_lossy(),
                "CORTEX_MCP_ECHO_LABEL": "a",
                "CORTEX_MCP_ECHO_EXTRA_TOOL": "b_c"
            }
        }
    }));
    let manager = McpManager::connect_with(
        servers,
        McpConnectOptions {
            bus: None,
            credentials: None,
            roots: Vec::new(),
            reserved_tool_ids: vec!["apply_patch".to_owned(), "plugin_tool".to_owned()],
        },
    )
    .await;

    let mut advertised = manager
        .capabilities()
        .into_iter()
        .map(|capability| capability.id)
        .collect::<Vec<_>>();
    let total = advertised.len();
    advertised.sort();
    advertised.dedup();
    assert_eq!(
        advertised.len(),
        total,
        "MCP model definitions must not contain duplicate ids"
    );
    assert_eq!(
        manager.route_for_id("apply_patch"),
        Some(CapabilityRoute::Builtin),
        "a reserved built-in id remains a built-in route"
    );
    assert_eq!(
        manager.route_for_id("plugin_tool"),
        Some(CapabilityRoute::Builtin),
        "an arbitrary surface-provided tool id remains reserved"
    );

    let patch = manager
        .tools()
        .into_iter()
        .find(|tool| tool.server == "apply" && tool.tool == "patch")
        .expect("collision fixture exposes apply/patch");
    assert_ne!(
        patch.id, "apply_patch",
        "the MCP tool cannot claim the reserved built-in id"
    );
    assert_eq!(
        manager.route_for_id(&patch.id),
        Some(CapabilityRoute::McpTool {
            server: "apply".to_owned(),
            tool: "patch".to_owned(),
        })
    );
    assert!(
        manager
            .call(&patch.id, json!({}))
            .await
            .expect("namespaced patch call")
            .content
            .contains("apply: collision tool patch")
    );

    let plugin_tool = manager
        .tools()
        .into_iter()
        .find(|tool| tool.server == "plugin" && tool.tool == "tool")
        .expect("collision fixture exposes plugin/tool");
    assert_ne!(
        plugin_tool.id, "plugin_tool",
        "an MCP tool cannot claim a plugin-reserved id"
    );
    assert!(
        manager
            .call(&plugin_tool.id, json!({}))
            .await
            .expect("namespaced plugin tool call")
            .content
            .contains("plugin: collision tool tool")
    );

    let resource_tool = manager
        .tools()
        .into_iter()
        .find(|tool| tool.server == "demo" && tool.tool == "resource_notes://demo")
        .expect("collision fixture tool");
    let resource = manager
        .resources()
        .into_iter()
        .find(|resource| resource.server == "demo" && resource.uri == "notes://demo")
        .expect("collision fixture resource");
    assert_ne!(
        resource_tool.id, resource.id,
        "a tool and a resource must never share a model-visible id"
    );
    assert!(
        manager
            .call(&resource_tool.id, json!({}))
            .await
            .expect("tool call")
            .content
            .contains("demo: collision tool resource_notes://demo")
    );
    assert!(
        manager
            .read_resource(&resource.id)
            .await
            .expect("resource read")
            .content
            .contains("RESOURCE-CONTENT-default")
    );

    let prompt_tool = manager
        .tools()
        .into_iter()
        .find(|tool| tool.server == "demo" && tool.tool == "prompt_greet")
        .expect("collision fixture prompt tool");
    let prompt = manager
        .prompts()
        .into_iter()
        .find(|prompt| prompt.server == "demo" && prompt.name == "greet")
        .expect("collision fixture prompt");
    assert_ne!(
        prompt_tool.id, prompt.id,
        "a tool and prompt must never share a model-visible id"
    );
    assert!(
        manager
            .call(&prompt_tool.id, json!({}))
            .await
            .expect("tool call")
            .content
            .contains("demo: collision tool prompt_greet")
    );
    assert!(
        manager
            .get_prompt(&prompt.id, json!({ "name": "route" }))
            .await
            .expect("prompt fetch")
            .content
            .contains("Hello route from MCP")
    );
    assert!(
        manager.read_resource("notes://demo").await.is_err(),
        "a raw resource URI is not a model-visible route and must not select a server by precedence"
    );
    assert!(
        manager.get_prompt("greet", json!({})).await.is_err(),
        "a raw prompt name is not a model-visible route and must not select a server by precedence"
    );

    let a_b = manager
        .tools()
        .into_iter()
        .find(|tool| tool.server == "a_b" && tool.tool == "c")
        .expect("a_b/c capability");
    let a = manager
        .tools()
        .into_iter()
        .find(|tool| tool.server == "a" && tool.tool == "b_c")
        .expect("a/b_c capability");
    assert_ne!(
        a_b.id, a.id,
        "sanitized cross-server identities must be disambiguated"
    );
    assert!(
        manager
            .call(&a_b.id, json!({}))
            .await
            .expect("a_b/c call")
            .content
            .contains("a_b: collision tool c")
    );
    assert!(
        manager
            .call(&a.id, json!({}))
            .await
            .expect("a/b_c call")
            .content
            .contains("a: collision tool b_c")
    );

    manager.shutdown().await;
    let log_contents = std::fs::read_to_string(&log).expect("fixture log");
    for expected in [
        "tools/call name=patch",
        "tools/call name=tool",
        "tools/call name=resource_notes://demo",
        "resources/read uri=notes://demo",
        "tools/call name=prompt_greet",
        "prompts/get name=greet",
        "tools/call name=c",
        "tools/call name=b_c",
    ] {
        assert!(
            log_contents.contains(expected),
            "fixture log is missing `{expected}`:\n{log_contents}"
        );
    }
    let _ = std::fs::remove_file(log);
}

fn tool_ids(manager: &McpManager) -> Vec<String> {
    manager.tools().iter().map(|t| t.id.clone()).collect()
}
