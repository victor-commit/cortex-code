//! Black-box audit sink coverage through the shipped `cortex run` binary.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
use std::sync::Mutex;
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

const RUN_TIMEOUT: Duration = Duration::from_secs(60);
static TEST_LOCK: Mutex<()> = Mutex::new(());

fn acquire_test_lock() -> std::sync::MutexGuard<'static, ()> {
    match TEST_LOCK.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

struct Scratch {
    root: PathBuf,
    home: PathBuf,
    project: PathBuf,
    db: PathBuf,
    audit: PathBuf,
}

impl Scratch {
    fn new(tag: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "cortex-audit-sink-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let home = root.join("home");
        let project = root.join("project");
        std::fs::create_dir_all(&home).expect("create home");
        std::fs::create_dir_all(&project).expect("create project");
        Self {
            db: root.join("data/cortex.db"),
            audit: root.join("audit.jsonl"),
            root,
            home,
            project,
        }
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

struct ScriptedProvider {
    base_url: String,
    thread: Option<thread::JoinHandle<()>>,
}

#[derive(Clone, Copy)]
enum ProviderScenario {
    CoreTools,
    McpTool,
    SecretBash,
    BlockedBash,
    SubagentTask,
}

impl ScriptedProvider {
    fn spawn() -> Self {
        Self::spawn_scenario(ProviderScenario::CoreTools)
    }

    fn spawn_mcp() -> Self {
        Self::spawn_scenario(ProviderScenario::McpTool)
    }

    fn spawn_secret_bash() -> Self {
        Self::spawn_scenario(ProviderScenario::SecretBash)
    }

    fn spawn_blocked_bash() -> Self {
        Self::spawn_scenario(ProviderScenario::BlockedBash)
    }

    fn spawn_subagent_task() -> Self {
        Self::spawn_scenario(ProviderScenario::SubagentTask)
    }

    fn spawn_scenario(scenario: ProviderScenario) -> Self {
        let (port, listener) = bind_provider_listener();
        listener
            .set_nonblocking(true)
            .expect("set nonblocking listener");
        let thread = thread::spawn(move || {
            let mut sent_primary_tool_turn = false;
            let mut sent_primary_final_turn = false;
            let mut sent_subagent_child_turn = false;
            let mut sent_parent_final_after_subagent = false;

            for _ in 0..10 {
                let Some(mut stream) = accept_with_idle_timeout(&listener, Duration::from_secs(4))
                else {
                    break;
                };
                let request_bytes = read_http_body(&mut stream);
                let request_text = String::from_utf8_lossy(&request_bytes);
                let request_has_tools = request_text.contains("\"tools\"");

                let body = match scenario {
                    ProviderScenario::CoreTools
                    | ProviderScenario::McpTool
                    | ProviderScenario::SecretBash
                    | ProviderScenario::BlockedBash => {
                        if !sent_primary_tool_turn && request_has_tools {
                            sent_primary_tool_turn = true;
                            match scenario {
                                ProviderScenario::CoreTools => tool_call_body(),
                                ProviderScenario::McpTool => mcp_tool_call_body(),
                                ProviderScenario::SecretBash => secret_bash_tool_call_body(),
                                ProviderScenario::BlockedBash => blocked_bash_tool_call_body(),
                                ProviderScenario::SubagentTask => unreachable!(),
                            }
                        } else {
                            if sent_primary_tool_turn {
                                sent_primary_final_turn = true;
                            }
                            final_body()
                        }
                    }
                    ProviderScenario::SubagentTask => {
                        if !sent_primary_tool_turn && request_has_tools {
                            sent_primary_tool_turn = true;
                            subagent_task_tool_call_body()
                        } else if sent_primary_tool_turn && !sent_subagent_child_turn {
                            sent_subagent_child_turn = true;
                            subagent_child_final_body()
                        } else {
                            sent_parent_final_after_subagent = true;
                            final_body()
                        }
                    }
                };
                write_sse_response(&mut stream, &body).expect("write provider response");

                if sent_primary_tool_turn {
                    if !matches!(scenario, ProviderScenario::SubagentTask) {
                        if sent_primary_final_turn {
                            break;
                        }
                        continue;
                    }
                    if sent_subagent_child_turn && sent_parent_final_after_subagent {
                        break;
                    }
                }
            }
        });
        Self {
            base_url: format!("http://127.0.0.1:{port}"),
            thread: Some(thread),
        }
    }
}

fn accept_with_idle_timeout(listener: &TcpListener, idle_timeout: Duration) -> Option<TcpStream> {
    let start = Instant::now();
    loop {
        match listener.accept() {
            Ok((stream, _)) => return Some(stream),
            Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                if start.elapsed() > idle_timeout {
                    return None;
                }
                thread::sleep(Duration::from_millis(20));
            }
            Err(err) => panic!("accept provider request: {err}"),
        }
    }
}

impl Drop for ScriptedProvider {
    fn drop(&mut self) {
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[test]
fn run_redacts_provider_auth_body_and_shell_env_secrets_in_audit_records() {
    let _guard = acquire_test_lock();
    let scratch = Scratch::new("redaction");
    let provider = ScriptedProvider::spawn_secret_bash();
    write_shell_env_plugin(&scratch.project);
    let key = fake_openrouter_key();
    let test_secret = "topsecret123";

    let out = run(cortex_with_key(
        &scratch,
        &provider,
        &[
            "run",
            "--format",
            "json",
            "--permission-allow",
            "bash:*",
            "exercise audit redaction for prompt secret topsecret123",
        ],
        &key,
    ));
    assert_success(&out);

    let raw = std::fs::read_to_string(&scratch.audit).expect("audit raw");
    assert!(
        !raw.contains(&key),
        "audit must not contain the OpenRouter key literal: {raw}"
    );
    assert!(
        !raw.contains(test_secret),
        "audit must not contain the plugin/test secret literal: {raw}"
    );
    assert!(
        raw.contains(redaction::PLACEHOLDER),
        "audit should mark redacted fields with the canonical placeholder: {raw}"
    );

    let records = read_audit(&scratch.audit);
    assert!(records.iter().any(|record| {
        record["type"] == "network_call"
            && record["kind"] == "provider_request"
            && record["provider"] == "openrouter"
            && record["model"] == "google/gemini-2.5-flash"
            && record["host"] == "127.0.0.1"
            && record["auth"] == redaction::PLACEHOLDER
            && record["body"] == redaction::PLACEHOLDER
            && record["headers"]["authorization"] == redaction::PLACEHOLDER
            && record["duration_ms"].as_u64().is_some()
            && record["outcome"] == "success"
    }));
    assert!(records.iter().any(|record| {
        record["type"] == "plugin_hook"
            && record["hook"] == "shell.env"
            && record["env"]["SECRET"] == redaction::PLACEHOLDER
            && record["env_keys"]
                .as_array()
                .is_some_and(|keys| keys.iter().any(|key| key == "SECRET"))
    }));
    let redacted_shell_command_seen = records.iter().any(|record| {
        record["type"] == "shell_execution"
            && record["command"]
                .as_str()
                .is_some_and(|command| command.contains(redaction::PLACEHOLDER))
    });
    if !redacted_shell_command_seen {
        assert!(
            records.iter().any(|record| {
                record["type"] == "permission_decision"
                    && record["tool"] == "bash"
                    && record["outcome"] == "denied"
            }),
            "expected either a redacted shell execution command or an explicit denied bash decision: {records:#?}"
        );
    }
}

#[test]
fn run_writes_parseable_audit_records_for_permission_shell_file_and_provider_network() {
    let _guard = acquire_test_lock();
    let scratch = Scratch::new("core");
    let provider = ScriptedProvider::spawn();

    let out = run(cortex(
        &scratch,
        &provider,
        &[
            "run",
            "--format",
            "json",
            "--permission-allow",
            "bash:ls *",
            "--permission-allow",
            "write:*",
            "--permission-allow-save",
            "bash:echo asked*",
            "--permission-deny",
            "bash:echo denied*",
            "exercise audit sink coverage",
        ],
    ));
    assert_success(&out);

    let doc: Value = serde_json::from_slice(&out.stdout).expect("run JSON");
    let session = doc["session"].as_str().expect("session id");
    assert!(scratch.project.join("audit.txt").exists());

    let records = read_audit(&scratch.audit);
    assert_schema_and_order(&records, Some(session));
    assert!(
        records.iter().all(|record| record["session"] == session),
        "every record must carry the run session id: {records:#?}"
    );
    assert!(records.iter().any(|record| {
        record["type"] == "permission_decision"
            && record["tool"] == "bash"
            && record["decision"] == "ask"
            && record["outcome"] == "allowed"
            && record["pattern"] == "bash:echo asked*".trim_start_matches("bash:")
    }));
    assert!(records.iter().any(|record| {
        record["type"] == "permission_decision"
            && record["tool"] == "bash"
            && record["decision"] == "deny"
            && record["outcome"] == "denied"
            && record["request"] == "echo denied-direct"
    }));
    assert!(records.iter().any(|record| {
        record["type"] == "shell_execution"
            && record["command"] == "echo asked-ok"
            && record["exit_status"] == 0
            && record["cwd"]
                .as_str()
                .is_some_and(|cwd| cwd.ends_with("project"))
    }));
    assert!(records.iter().any(|record| {
        record["type"] == "shell_execution"
            && record["command"] == "ls /no/such/dir"
            && record["exit_status"].as_i64().is_some_and(|code| code != 0)
    }));
    assert!(records.iter().any(|record| {
        record["type"] == "file_mutation"
            && record["operation"] == "write"
            && record["outcome"] == "success"
            && record["path"]
                .as_str()
                .is_some_and(|path| path.ends_with("audit.txt"))
    }));
    assert!(records.iter().any(|record| {
        record["type"] == "network_call"
            && record["actor"] == "llm"
            && record["provider"] == "openrouter"
            && record["method"] == "POST"
            && record["host"] == "127.0.0.1"
            && record["endpoint"]
                .as_str()
                .is_some_and(|endpoint| endpoint.ends_with("/chat/completions"))
    }));
    assert_eq!(
        count_records(&records, |record| {
            record["type"] == "permission_decision"
                && record["tool"] == "bash"
                && record["request"] == "echo asked-ok"
        }),
        1,
        "allowed permission should be audited exactly once: {records:#?}"
    );
    assert_eq!(
        count_records(&records, |record| {
            record["type"] == "permission_decision"
                && record["tool"] == "bash"
                && record["request"] == "echo denied-direct"
        }),
        1,
        "denied permission should be audited exactly once: {records:#?}"
    );
    assert_eq!(
        count_records(&records, |record| {
            record["type"] == "shell_execution" && record["command"] == "echo asked-ok"
        }),
        1,
        "successful shell execution should be audited exactly once: {records:#?}"
    );
    assert_eq!(
        count_records(&records, |record| {
            record["type"] == "shell_execution" && record["command"] == "ls /no/such/dir"
        }),
        1,
        "failing shell execution should be audited exactly once: {records:#?}"
    );
    assert_eq!(
        count_records(&records, |record| {
            record["type"] == "file_mutation"
                && record["operation"] == "write"
                && record["path"]
                    .as_str()
                    .is_some_and(|path| path.ends_with("audit.txt"))
        }),
        1,
        "file write mutation should be audited exactly once: {records:#?}"
    );
}

#[test]
fn denied_shell_effect_is_audited_without_execution_or_disk_effect() {
    let _guard = acquire_test_lock();
    let scratch = Scratch::new("deny");
    let provider = ScriptedProvider::spawn_blocked_bash();

    let out = run(cortex(
        &scratch,
        &provider,
        &[
            "run",
            "--format",
            "json",
            "--permission-deny",
            "bash:touch denied-sentinel.txt",
            "exercise denied shell audit",
        ],
    ));
    assert_success(&out);

    let doc: Value = serde_json::from_slice(&out.stdout).expect("run JSON");
    let session = doc["session"].as_str().expect("session id");
    assert!(
        !scratch.project.join("denied-sentinel.txt").exists(),
        "denied command must not touch the filesystem"
    );

    let records = read_audit(&scratch.audit);
    assert_schema_and_order(&records, Some(session));
    assert_eq!(
        count_records(&records, |record| {
            record["type"] == "permission_decision"
                && record["tool"] == "bash"
                && record["request"] == "touch denied-sentinel.txt"
                && record["outcome"] == "denied"
        }),
        1,
        "denial should be audited exactly once: {records:#?}"
    );
    assert_eq!(
        count_records(&records, |record| {
            record["type"] == "shell_execution" && record["command"] == "touch denied-sentinel.txt"
        }),
        0,
        "blocked shell command must not have an execution record: {records:#?}"
    );
}

#[test]
fn no_net_refusal_is_audited_as_blocked_network_attempt() {
    let _guard = acquire_test_lock();
    let scratch = Scratch::new("no-net");
    let mut command = cortex_without_provider(
        &scratch,
        &["run", "--format", "json", "exercise no-net audit"],
    );
    command.env("OPENROUTER_API_KEY", fake_openrouter_key());
    command.env("CORTEX_NO_NET", "1");

    let out = run(command);
    assert_eq!(
        out.status.code(),
        Some(4),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(out.stdout.is_empty(), "no-net refusal keeps stdout clean");

    let records = read_audit(&scratch.audit);
    assert_schema_and_order(&records, None);
    assert_eq!(
        count_records(&records, |record| {
            record["type"] == "network_call"
                && record["actor"] == "llm"
                && record["outcome"] == "net-disabled"
        }),
        1,
        "no-net refusal should be one blocked network record: {records:#?}"
    );
    assert_eq!(
        count_records(&records, |record| {
            record["type"] == "network_call" && record["outcome"] == "success"
        }),
        0,
        "no-net refusal must not claim a successful network call: {records:#?}"
    );
}

#[test]
fn pure_mode_keeps_core_audit_records_without_plugin_chatter() {
    let _guard = acquire_test_lock();
    let scratch = Scratch::new("pure");
    let provider = ScriptedProvider::spawn();
    write_shell_env_plugin(&scratch.project);

    let out = run(cortex(
        &scratch,
        &provider,
        &[
            "run",
            "--pure",
            "--format",
            "json",
            "--permission-allow",
            "bash:ls *",
            "--permission-allow",
            "write:*",
            "--permission-allow-save",
            "bash:echo asked*",
            "--permission-deny",
            "bash:echo denied*",
            "exercise pure audit",
        ],
    ));
    assert_success(&out);

    let doc: Value = serde_json::from_slice(&out.stdout).expect("run JSON");
    let session = doc["session"].as_str().expect("session id");
    let records = read_audit(&scratch.audit);
    assert_schema_and_order(&records, Some(session));
    assert!(
        records
            .iter()
            .any(|record| record["type"] == "permission_decision")
    );
    assert!(
        records
            .iter()
            .any(|record| record["type"] == "shell_execution")
    );
    assert!(
        records
            .iter()
            .any(|record| record["type"] == "file_mutation")
    );
    assert!(
        records
            .iter()
            .any(|record| { record["type"] == "network_call" && record["actor"] == "llm" })
    );
    assert_eq!(
        count_records(&records, |record| {
            record["type"] == "plugin_load" || record["type"] == "plugin_hook"
        }),
        0,
        "--pure should suppress plugin records only: {records:#?}"
    );
}

#[test]
fn run_writes_mcp_tool_call_audit_record_with_server_tool_and_arguments() {
    let _guard = acquire_test_lock();
    let scratch = Scratch::new("mcp");
    let provider = ScriptedProvider::spawn_mcp();
    let server = echo_server_bin();
    let config = format!(
        "[mcp.demo]\ncommand = [\"{}\"]\n",
        server.display().to_string().replace('\\', "\\\\")
    );
    std::fs::write(scratch.project.join("cortex.toml"), config).expect("write mcp config");

    let out = run(cortex(
        &scratch,
        &provider,
        &[
            "run",
            "--format",
            "json",
            "--permission-allow",
            "demo_add:demo_add",
            "exercise mcp audit",
        ],
    ));
    assert_success(&out);

    let records = read_audit(&scratch.audit);
    assert!(records.iter().any(|record| {
        record["type"] == "mcp_tool_call"
            && record["server"] == "demo"
            && record["tool"] == "add"
            && record["qualified_tool"] == "demo_add"
            && record["arguments_summary"]
                .as_str()
                .is_some_and(|summary| summary.contains("a,b"))
            && record["outcome"] == "success"
    }));
}

#[test]
fn subagent_provider_requests_are_attributed_to_the_child_session() {
    let _guard = acquire_test_lock();
    let scratch = Scratch::new("subagent-provider-attribution");
    let provider = ScriptedProvider::spawn_subagent_task();

    let out = run(cortex(
        &scratch,
        &provider,
        &[
            "run",
            "--format",
            "json",
            "--permission-allow",
            "task:explore",
            "exercise subagent provider attribution",
        ],
    ));
    assert_success(&out);

    let doc: Value = serde_json::from_slice(&out.stdout).expect("run JSON");
    let parent = doc["session"].as_str().expect("parent session");
    let child = child_session_id(&tool_result_text(&doc, "task"));
    assert_ne!(child, parent, "task tool should report a child session");

    let records = read_audit(&scratch.audit);
    assert_schema_and_order(&records, None);

    assert!(
        records.iter().any(|record| {
            record["type"] == "network_call"
                && record["kind"] == "provider_request"
                && record["session"] == parent
        }),
        "parent turn should still emit provider_request audit records: {records:#?}"
    );

    assert!(
        records.iter().any(|record| {
            record["type"] == "network_call"
                && record["kind"] == "provider_request"
                && record["session"] == child
                && record["parent_session"] == parent
                && record["parent_session_id"] == parent
        }),
        "child turn should emit provider_request records attributed to the child session: {records:#?}"
    );
}

#[test]
fn audit_records_survive_restart_and_debug_reader_returns_ordered_json() {
    let _guard = acquire_test_lock();
    let scratch = Scratch::new("restart");
    let provider = ScriptedProvider::spawn();
    let session = format!("audit-restart-{}", std::process::id());

    let out = run(cortex(
        &scratch,
        &provider,
        &[
            "run",
            "--session",
            &session,
            "--format",
            "json",
            "--permission-allow",
            "bash:ls *",
            "--permission-allow",
            "write:*",
            "exercise durable audit restart",
        ],
    ));
    assert_success(&out);
    drop(provider);

    let before_restart = read_audit(&scratch.audit);
    assert_schema_and_order(&before_restart, Some(&session));
    let last_sequence = before_restart
        .last()
        .and_then(|record| record["sequence"].as_u64())
        .expect("last sequence");

    let reader = run(cortex_without_provider(
        &scratch,
        &["debug", "audit", "--format", "json", "--limit", "0"],
    ));
    assert_success(&reader);
    let after_restart: Vec<Value> =
        serde_json::from_slice(&reader.stdout).expect("restart audit JSON");
    assert_eq!(
        after_restart.len(),
        before_restart.len(),
        "separate reader must return all durable records"
    );
    assert_schema_and_order(&after_restart, Some(&session));
    assert_eq!(
        after_restart
            .last()
            .and_then(|record| record["sequence"].as_u64()),
        Some(last_sequence)
    );
}

#[test]
fn cross_m5_single_turn_observes_mcp_plugin_permission_and_audit() {
    let _guard = acquire_test_lock();
    if plugins::detect_runtime().is_none() {
        eprintln!("skipping cross-M5 proof: no Bun, Node, or Deno on PATH");
        return;
    }

    let scratch = Scratch::new("cross-m5");
    let provider = ScriptedProvider::spawn_mcp();
    let server = echo_server_bin();
    let session = format!("cross-m5-single-turn-{}", std::process::id());
    let plugin_marker = scratch.project.join("plugin-events.jsonl");
    let mcp_log = scratch.project.join("mcp-echo.log");
    write_cross_m5_plugin_and_mcp_config(
        &scratch.project,
        &server,
        &plugin_marker,
        &mcp_log,
        &session,
    );

    let mut command = cortex(
        &scratch,
        &provider,
        &[
            "run",
            "--session",
            &session,
            "--format",
            "json",
            "--permission-allow-save",
            "demo_add:demo_add",
            "exercise the full extensibility stack in one turn",
        ],
    );
    command.env("CORTEX_VERBOSE", "1");
    let out = run(command);
    assert_success(&out);

    let doc: Value = serde_json::from_slice(&out.stdout).expect("run JSON");
    assert_eq!(doc["session"], session);

    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("permission.asked") && stderr.contains("permission.replied"),
        "verbose stderr should expose the permission.* decision events: {stderr}"
    );

    let plugin_events = read_json_lines(&plugin_marker);
    assert!(
        plugin_events.iter().all(|event| event["run"] == session),
        "plugin marker lines should be attributed to the fixed run id: {plugin_events:#?}"
    );
    assert!(
        plugin_events
            .iter()
            .any(|event| event["kind"] == "lifecycle")
    );
    assert!(
        plugin_events
            .iter()
            .any(|event| event["kind"] == "permission.asked")
            && plugin_events
                .iter()
                .any(|event| event["kind"] == "permission.replied")
            && plugin_events
                .iter()
                .any(|event| event["kind"] == "tool.result"),
        "plugin event hook must observe the gated tool lifecycle: {plugin_events:#?}"
    );

    let records = read_audit(&scratch.audit);
    assert_schema_and_order(&records, Some(&session));
    assert!(records.iter().any(|record| {
        record["type"] == "plugin_load"
            && record["session"] == session
            && record["hooks"]
                .as_array()
                .is_some_and(|hooks| hooks.iter().any(|hook| hook == "event"))
    }));
    assert!(records.iter().any(|record| {
        record["type"] == "permission_decision"
            && record["session"] == session
            && record["tool"] == "demo_add"
            && record["decision"] == "ask"
            && record["outcome"] == "allowed"
    }));
    assert!(records.iter().any(|record| {
        record["type"] == "mcp_tool_call"
            && record["session"] == session
            && record["server"] == "demo"
            && record["tool"] == "add"
            && record["qualified_tool"] == "demo_add"
            && record["outcome"] == "success"
    }));

    let mcp_log_text = std::fs::read_to_string(&mcp_log).expect("MCP log");
    assert!(
        mcp_log_text.contains("tools/call name=add"),
        "MCP fixture must receive a real tools/call: {mcp_log_text}"
    );
    assert_pid_reaped(mcp_pid_from_log(&mcp_log_text), "MCP echo server");
    for pid in plugin_pids(&plugin_events) {
        assert_pid_reaped(pid, "plugin runtime");
    }
}

fn cortex(scratch: &Scratch, provider: &ScriptedProvider, args: &[&str]) -> Command {
    cortex_with_key(scratch, provider, args, &fake_openrouter_key())
}

fn cortex_with_key(
    scratch: &Scratch,
    provider: &ScriptedProvider,
    args: &[&str],
    key: &str,
) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_cortex"));
    command.current_dir(&scratch.project);
    command.args(args);
    command.env("HOME", &scratch.home);
    command.env("CORTEX_DB_PATH", &scratch.db);
    command.env("CORTEX_AUDIT_LOG", &scratch.audit);
    command.env("OPENROUTER_API_KEY", key);
    command.env("CORTEX_OPENROUTER_BASE_URL", &provider.base_url);
    command
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("XDG_DATA_HOME")
        .env_remove("CORTEX_DATA_DIR")
        .env_remove("CORTEX_AUTH_FILE")
        .env_remove("CORTEX_NO_NET")
        .env_remove("RUST_LOG")
        .env_remove("CORTEX_VERBOSE")
        .env_remove("NO_COLOR");
    command
}

fn cortex_without_provider(scratch: &Scratch, args: &[&str]) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_cortex"));
    command.current_dir(&scratch.project);
    command.args(args);
    command.env("HOME", &scratch.home);
    command.env("CORTEX_DB_PATH", &scratch.db);
    command.env("CORTEX_AUDIT_LOG", &scratch.audit);
    command
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("XDG_DATA_HOME")
        .env_remove("CORTEX_DATA_DIR")
        .env_remove("CORTEX_AUTH_FILE")
        .env_remove("OPENROUTER_API_KEY")
        .env_remove("CORTEX_OPENROUTER_BASE_URL")
        .env_remove("CORTEX_NO_NET")
        .env_remove("RUST_LOG")
        .env_remove("CORTEX_VERBOSE")
        .env_remove("NO_COLOR");
    command
}

fn run(mut command: Command) -> Output {
    command.stdin(Stdio::null());
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());
    let mut child = command.spawn().expect("spawn cortex binary");
    let mut stdout = child.stdout.take().expect("stdout");
    let mut stderr = child.stderr.take().expect("stderr");
    let stdout_thread = thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = stdout.read_to_end(&mut bytes);
        bytes
    });
    let stderr_thread = thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = stderr.read_to_end(&mut bytes);
        bytes
    });
    let start = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().expect("try wait") {
            break status;
        }
        if start.elapsed() > RUN_TIMEOUT {
            let _ = child.kill();
            let _ = child.wait();
            panic!("cortex run timed out");
        }
        thread::sleep(Duration::from_millis(20));
    };
    Output {
        status,
        stdout: stdout_thread.join().expect("stdout join"),
        stderr: stderr_thread.join().expect("stderr join"),
    }
}

fn assert_success(out: &Output) {
    assert_eq!(
        out.status.code(),
        Some(0),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

fn read_audit(path: &std::path::Path) -> Vec<Value> {
    let text = std::fs::read_to_string(path).expect("audit file");
    text.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str(line).expect("audit JSONL record"))
        .collect()
}

fn read_json_lines(path: &std::path::Path) -> Vec<Value> {
    let text = std::fs::read_to_string(path).expect("JSONL file");
    text.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str(line).expect("JSONL record"))
        .collect()
}

fn assert_schema_and_order(records: &[Value], session: Option<&str>) {
    assert!(!records.is_empty(), "audit sink should contain records");
    let mut previous_sequence = 0_u64;
    let mut previous_timestamp = 0_u64;
    let mut turn_id: Option<&str> = None;
    for record in records {
        assert_eq!(record["schema_version"], 1, "{record:#?}");
        assert!(
            record["type"]
                .as_str()
                .is_some_and(|value| !value.is_empty())
        );
        assert!(
            record["outcome"]
                .as_str()
                .is_some_and(|value| !value.is_empty())
        );
        assert!(
            record["session"]
                .as_str()
                .is_some_and(|value| !value.is_empty())
        );
        assert_eq!(record["session_id"], record["session"], "{record:#?}");
        if let Some(expected) = session {
            assert_eq!(record["session"], expected, "{record:#?}");
            match turn_id {
                Some(existing) => assert_eq!(record["turn_id"], existing, "{record:#?}"),
                None => turn_id = record["turn_id"].as_str(),
            }
        } else {
            assert!(
                record["turn_id"]
                    .as_str()
                    .is_some_and(|value| !value.is_empty())
            );
        }
        let sequence = record["sequence"].as_u64().expect("numeric sequence");
        assert!(
            sequence > previous_sequence,
            "sequence must be strictly monotonic: {records:#?}"
        );
        previous_sequence = sequence;
        let timestamp = record["timestamp_ms"].as_u64().expect("numeric timestamp");
        assert_eq!(record["timestamp"], record["timestamp_ms"], "{record:#?}");
        assert!(
            timestamp >= previous_timestamp,
            "timestamps should not go backwards: {records:#?}"
        );
        previous_timestamp = timestamp;
    }
}

fn count_records<F>(records: &[Value], mut predicate: F) -> usize
where
    F: FnMut(&Value) -> bool,
{
    records.iter().filter(|record| predicate(record)).count()
}

fn tool_result_text(doc: &serde_json::Value, tool_name: &str) -> String {
    doc["tool_results"]
        .as_array()
        .expect("tool results")
        .iter()
        .find(|result| result["name"] == tool_name)
        .expect("target tool result")
        .get("parts")
        .and_then(serde_json::Value::as_array)
        .expect("parts")
        .iter()
        .filter_map(|part| part.get("text").and_then(serde_json::Value::as_str))
        .collect::<Vec<_>>()
        .join("\n")
}

fn child_session_id(task_text: &str) -> String {
    let (_, rest) = task_text
        .split_once("completed in session `")
        .expect("task result names the child session");
    let (id, _) = rest.split_once('`').expect("session id is backtick-closed");
    id.to_owned()
}

fn tool_call_body() -> String {
    let calls = [
        ("call_ask", "bash", json!({ "command": "echo asked-ok" })),
        (
            "call_deny",
            "bash",
            json!({ "command": "echo denied-direct" }),
        ),
        ("call_fail", "bash", json!({ "command": "ls /no/such/dir" })),
        (
            "call_write",
            "write",
            json!({ "path": "audit.txt", "content": "audited\n" }),
        ),
    ];
    let tool_calls = calls
        .into_iter()
        .enumerate()
        .map(|(index, (id, name, arguments))| {
            json!({
                "index": index,
                "id": id,
                "type": "function",
                "function": {
                    "name": name,
                    "arguments": arguments.to_string()
                }
            })
        })
        .collect::<Vec<_>>();
    sse_body(&[
        json!({
            "choices": [{
                "delta": { "tool_calls": tool_calls },
                "finish_reason": null
            }]
        })
        .to_string(),
        json!({
            "choices": [{
                "delta": {},
                "finish_reason": "tool_calls"
            }]
        })
        .to_string(),
    ])
}

fn mcp_tool_call_body() -> String {
    sse_body(&[
        json!({
            "choices": [{
                "delta": {
                    "tool_calls": [{
                        "index": 0,
                        "id": "call_mcp",
                        "type": "function",
                        "function": {
                            "name": "demo_add",
                            "arguments": json!({ "a": 21, "b": 21 }).to_string()
                        }
                    }]
                },
                "finish_reason": null
            }]
        })
        .to_string(),
        json!({
            "choices": [{
                "delta": {},
                "finish_reason": "tool_calls"
            }]
        })
        .to_string(),
    ])
}

fn secret_bash_tool_call_body() -> String {
    sse_body(&[
        json!({
            "choices": [{
                "delta": {
                    "tool_calls": [{
                        "index": 0,
                        "id": "call_secret_bash",
                        "type": "function",
                        "function": {
                            "name": "bash",
                            "arguments": json!({
                                "command": "printf '%s\\n' \"$SECRET\" && printf '%s\\n' \"topsecret123\""
                            }).to_string()
                        }
                    }]
                },
                "finish_reason": null
            }]
        })
        .to_string(),
        json!({
            "choices": [{
                "delta": {},
                "finish_reason": "tool_calls"
            }]
        })
        .to_string(),
    ])
}

fn blocked_bash_tool_call_body() -> String {
    sse_body(&[
        json!({
            "choices": [{
                "delta": {
                    "tool_calls": [{
                        "index": 0,
                        "id": "call_blocked_bash",
                        "type": "function",
                        "function": {
                            "name": "bash",
                            "arguments": json!({ "command": "touch denied-sentinel.txt" }).to_string()
                        }
                    }]
                },
                "finish_reason": null
            }]
        })
        .to_string(),
        json!({
            "choices": [{
                "delta": {},
                "finish_reason": "tool_calls"
            }]
        })
        .to_string(),
    ])
}

fn subagent_task_tool_call_body() -> String {
    sse_body(&[
        json!({
            "choices": [{
                "delta": {
                    "tool_calls": [{
                        "index": 0,
                        "id": "call_subagent",
                        "type": "function",
                        "function": {
                            "name": "task",
                            "arguments": json!({
                                "agent": "explore",
                                "prompt": "Reply with CHILD-OK and nothing else."
                            }).to_string()
                        }
                    }]
                },
                "finish_reason": null
            }]
        })
        .to_string(),
        json!({
            "choices": [{
                "delta": {},
                "finish_reason": "tool_calls"
            }]
        })
        .to_string(),
    ])
}

fn subagent_child_final_body() -> String {
    sse_body(&[
        json!({
            "choices": [{
                "delta": { "content": "CHILD-OK" },
                "finish_reason": null
            }]
        })
        .to_string(),
        json!({
            "choices": [{
                "delta": {},
                "finish_reason": "stop"
            }],
            "usage": {
                "prompt_tokens": 2,
                "completion_tokens": 1,
                "total_tokens": 3
            }
        })
        .to_string(),
    ])
}

fn final_body() -> String {
    sse_body(&[
        json!({
            "choices": [{
                "delta": { "content": "audit complete" },
                "finish_reason": null
            }]
        })
        .to_string(),
        json!({
            "choices": [{
                "delta": {},
                "finish_reason": "stop"
            }],
            "usage": {
                "prompt_tokens": 3,
                "completion_tokens": 2,
                "total_tokens": 5
            }
        })
        .to_string(),
    ])
}

fn write_shell_env_plugin(project: &std::path::Path) {
    std::fs::write(
        project.join("plugin.mjs"),
        r#"
export default async function plugin() {
  return {
    "shell.env": async () => {
      return { SECRET: "topsecret123", VISIBLE_FLAG: "present" };
    }
  };
}
"#,
    )
    .expect("write plugin");
    std::fs::write(project.join("cortex.toml"), "plugin = [\"./plugin.mjs\"]\n")
        .expect("write config");
}

fn write_cross_m5_plugin_and_mcp_config(
    project: &std::path::Path,
    server: &std::path::Path,
    marker: &std::path::Path,
    mcp_log: &std::path::Path,
    session: &str,
) {
    std::fs::write(
        project.join("plugin.mjs"),
        r#"
import { appendFile } from "node:fs/promises";

export default async function plugin(_input, options) {
  return {
    async event(event) {
      await appendFile(options.marker, JSON.stringify({
        run: options.run,
        kind: event.kind ?? event.type ?? "unknown",
        tool: event.tool ?? event.name ?? null,
        pid: process.pid
      }) + "\n");
    }
  };
}
"#,
    )
    .expect("write cross-M5 plugin");

    std::fs::write(
        project.join("cortex.toml"),
        format!(
            r#"plugin = [["./plugin.mjs", {{ marker = "{}", run = "{}" }}]]

[mcp.demo]
command = ["{}"]

[mcp.demo.environment]
CORTEX_MCP_ECHO_LOG = "{}"
"#,
            marker.display(),
            session,
            server.display(),
            mcp_log.display()
        ),
    )
    .expect("write cross-M5 config");
}

fn sse_body(frames: &[String]) -> String {
    let mut body = String::new();
    for frame in frames {
        body.push_str("data: ");
        body.push_str(frame);
        body.push_str("\n\n");
    }
    body.push_str("data: [DONE]\n\n");
    body
}

fn write_sse_response(stream: &mut TcpStream, body: &str) -> std::io::Result<()> {
    let response = format!(
        "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncache-control: no-cache\r\nconnection: close\r\ncontent-length: {}\r\n\r\n{}",
        body.len(),
        body
    );
    stream.write_all(response.as_bytes())?;
    stream.flush()
}

fn read_http_body(stream: &mut TcpStream) -> Vec<u8> {
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .expect("set timeout");
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 1024];
    let header_end = loop {
        let read = stream.read(&mut chunk).expect("read request");
        if read == 0 {
            return buffer;
        }
        buffer.extend_from_slice(&chunk[..read]);
        if let Some(end) = find_header_end(&buffer) {
            break end;
        }
    };
    let headers = String::from_utf8_lossy(&buffer[..header_end]);
    let content_length = headers
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().ok())
                .flatten()
        })
        .unwrap_or(0);
    let already = buffer.len().saturating_sub(header_end);
    let mut remaining = content_length.saturating_sub(already);
    while remaining > 0 {
        let read = stream.read(&mut chunk).expect("read body");
        if read == 0 {
            break;
        }
        buffer.extend_from_slice(&chunk[..read]);
        remaining = remaining.saturating_sub(read);
    }
    buffer
}

fn find_header_end(bytes: &[u8]) -> Option<usize> {
    bytes
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|pos| pos + 4)
}

fn fake_openrouter_key() -> String {
    format!("sk-{}-{}", "or-v1", "abcdef01".repeat(8))
}

fn echo_server_bin() -> PathBuf {
    let current = std::env::current_exe().expect("current test exe");
    let debug_dir = current
        .parent()
        .and_then(|deps| deps.parent())
        .expect("target debug dir");
    let bin = debug_dir.join("cortex-mcp-echo-server");
    if bin.exists() {
        return bin;
    }

    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let workspace = manifest_dir
        .parent()
        .and_then(|crates| crates.parent())
        .expect("workspace root");
    let status = Command::new(env!("CARGO"))
        .current_dir(workspace)
        .args(["build", "-p", "mcp", "--bin", "cortex-mcp-echo-server"])
        .status()
        .expect("build MCP echo fixture");
    assert!(status.success(), "failed to build MCP echo fixture");
    assert!(
        bin.exists(),
        "MCP echo fixture binary was not produced at {}",
        bin.display()
    );
    bin
}

fn mcp_pid_from_log(log: &str) -> u32 {
    log.lines()
        .find_map(|line| {
            let (_, rest) = line.split_once("started pid=")?;
            let pid = rest.split_whitespace().next()?;
            pid.parse().ok()
        })
        .expect("MCP startup pid in log")
}

fn plugin_pids(events: &[Value]) -> Vec<u32> {
    let mut pids = events
        .iter()
        .filter_map(|event| event["pid"].as_u64())
        .filter_map(|pid| u32::try_from(pid).ok())
        .collect::<Vec<_>>();
    pids.sort_unstable();
    pids.dedup();
    pids
}

fn assert_pid_reaped(pid: u32, label: &str) {
    let proc_path = std::path::PathBuf::from(format!("/proc/{pid}"));
    for _ in 0..40 {
        if !proc_path.exists() {
            return;
        }
        thread::sleep(Duration::from_millis(25));
    }
    panic!("{label} process pid {pid} is still alive");
}

fn bind_provider_listener() -> (u16, TcpListener) {
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind ephemeral loopback listener");
    let port = listener.local_addr().expect("local addr").port();
    (port, listener)
}
