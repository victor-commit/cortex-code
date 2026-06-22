//! Black-box audit sink coverage through the shipped `cortex run` binary.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU16, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

const RUN_TIMEOUT: Duration = Duration::from_secs(60);
static NEXT_PORT: AtomicU16 = AtomicU16::new(3280);

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

    fn spawn_scenario(scenario: ProviderScenario) -> Self {
        let (port, listener) = bind_provider_listener();
        listener
            .set_nonblocking(true)
            .expect("set nonblocking listener");
        let thread = thread::spawn(move || {
            for round in 0..2 {
                let mut stream = accept_with_timeout(&listener);
                let _ = read_http_body(&mut stream);
                let body = match (scenario, round) {
                    (ProviderScenario::CoreTools, 0) => tool_call_body(),
                    (ProviderScenario::McpTool, 0) => mcp_tool_call_body(),
                    (ProviderScenario::SecretBash, 0) => secret_bash_tool_call_body(),
                    (ProviderScenario::BlockedBash, 0) => blocked_bash_tool_call_body(),
                    _ => final_body(),
                };
                write_sse_response(&mut stream, &body).expect("write provider response");
            }
        });
        Self {
            base_url: format!("http://127.0.0.1:{port}"),
            thread: Some(thread),
        }
    }
}

fn accept_with_timeout(listener: &TcpListener) -> TcpStream {
    let start = Instant::now();
    loop {
        match listener.accept() {
            Ok((stream, _)) => return stream,
            Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                assert!(
                    start.elapsed() <= RUN_TIMEOUT,
                    "provider fixture timed out waiting for request"
                );
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
    assert!(records.iter().any(|record| {
        record["type"] == "shell_execution"
            && record["command"]
                .as_str()
                .is_some_and(|command| command.contains(redaction::PLACEHOLDER))
    }));
}

#[test]
fn run_writes_parseable_audit_records_for_permission_shell_file_and_provider_network() {
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

fn bind_provider_listener() -> (u16, TcpListener) {
    for _ in 0..40 {
        let port = NEXT_PORT.fetch_add(1, Ordering::SeqCst);
        if port > 3299 {
            NEXT_PORT.store(3280, Ordering::SeqCst);
            continue;
        }
        let addr: SocketAddr = format!("127.0.0.1:{port}").parse().unwrap();
        if let Ok(listener) = TcpListener::bind(addr) {
            return (port, listener);
        }
    }
    panic!("no free port in 3280-3299");
}
