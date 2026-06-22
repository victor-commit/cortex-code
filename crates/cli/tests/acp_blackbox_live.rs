use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

#[test]
#[ignore = "live OpenRouter black-box proof for cortex acp"]
fn acp_stdio_live_handshake_prompt_stream_and_cancel() {
    let Some(key) = live_key() else {
        eprintln!("skipping live ACP proof: OPENROUTER_API_KEY is not set");
        return;
    };

    let workspace = workspace_root();
    let scratch = Scratch::new("cortex-acp-live");
    let mut child = Command::new(env!("CARGO_BIN_EXE_cortex"))
        .arg("acp")
        .current_dir(&workspace)
        .env("OPENROUTER_API_KEY", key)
        .env("CORTEX_DB_PATH", scratch.path().join("cortex.db"))
        .env_remove("CORTEX_NO_NET")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn cortex acp");
    let pid = child.id();
    println!("ACP_PID={pid}");
    assert_no_tcp_listener(pid);

    let stdout = child.stdout.take().expect("stdout");
    let (tx, rx) = mpsc::channel::<Value>();
    let _reader = std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            let line = line.expect("read stdout line");
            if line.trim().is_empty() {
                continue;
            }
            let value: Value = serde_json::from_str(&line)
                .unwrap_or_else(|err| json!({ "_parse_error": err.to_string(), "_raw": line }));
            tx.send(value).ok();
        }
    });

    let mut messages = Vec::new();

    send(
        &mut child,
        1,
        "initialize",
        json!({
            "protocolVersion": 1,
            "clientCapabilities": {},
            "clientInfo": { "name": "factory-validator", "version": "0" }
        }),
    );
    let initialize = wait_for(&mut child, &rx, &mut messages, response_id(1), 15);
    assert!(initialize.get("error").is_none(), "{initialize}");
    assert_eq!(initialize["result"]["protocolVersion"], 1);
    assert!(initialize["result"]["agentCapabilities"].is_object());
    println!("initialize_ok protocolVersion=1 capabilities=present");

    send(
        &mut child,
        2,
        "session/new",
        json!({ "cwd": workspace, "mcpServers": [] }),
    );
    let new_session = wait_for(&mut child, &rx, &mut messages, response_id(2), 15);
    let session_id = new_session["result"]["sessionId"]
        .as_str()
        .expect("session id")
        .to_owned();
    assert!(!session_id.is_empty());
    println!("session_new_ok sessionId={session_id}");

    let nonce = fresh_uuid_like_nonce();
    let start = messages.len();
    send(
        &mut child,
        3,
        "session/prompt",
        json!({
            "sessionId": session_id,
            "prompt": [{ "type": "text", "text": format!("Reply with exactly this token, nothing else: {nonce}") }]
        }),
    );
    let prompt_final = wait_for(&mut child, &rx, &mut messages, response_id(3), 120);
    let chunks = update_texts(&messages[start..]);
    let combined = chunks.concat();
    assert!(!chunks.is_empty(), "expected at least one session/update");
    assert!(
        combined.contains(&nonce),
        "nonce {nonce} missing from streamed chunks: {combined:?}"
    );
    assert_eq!(prompt_final["result"]["stopReason"], "end_turn");
    println!(
        "prompt_live_ok chunks={} nonce_seen=true stop=end_turn",
        chunks.len()
    );

    let start = messages.len();
    send(
        &mut child,
        4,
        "session/prompt",
        json!({
            "sessionId": session_id,
            "prompt": [{ "type": "text", "text": "Count from 1 to 20, one number per line. Do not add any other words." }]
        }),
    );
    let long_final = wait_for(&mut child, &rx, &mut messages, response_id(4), 120);
    let long_chunks = update_texts(&messages[start..]);
    let long_text = long_chunks.concat();
    assert!(
        long_chunks.len() >= 2,
        "expected incremental streaming, got {} chunk(s): {long_text:?}",
        long_chunks.len()
    );
    assert!(long_text.contains('1') && long_text.contains("20"));
    assert_eq!(long_final["result"]["stopReason"], "end_turn");
    println!(
        "prompt_streaming_ok chunks={} reconstructed_contains_1_and_20=true",
        long_chunks.len()
    );

    let start = messages.len();
    send(
        &mut child,
        5,
        "session/prompt",
        json!({
            "sessionId": session_id,
            "prompt": [{ "type": "text", "text": "Write 800 numbered lines, one per line, starting at 1. Do not summarize." }]
        }),
    );
    wait_for(
        &mut child,
        &rx,
        &mut messages,
        |messages| !update_texts(&messages[start..]).is_empty(),
        45,
    );
    send_notification(
        &mut child,
        "session/cancel",
        json!({ "sessionId": session_id }),
    );
    let cancelled = wait_for(&mut child, &rx, &mut messages, response_id(5), 20);
    assert_eq!(cancelled["result"]["stopReason"], "cancelled");
    let chunks_at_cancel = update_texts(&messages[start..]).len();
    std::thread::sleep(Duration::from_secs(3));
    drain_messages(&rx, &mut messages);
    let chunks_later = update_texts(&messages[start..]).len();
    assert_eq!(
        chunks_later, chunks_at_cancel,
        "assistant chunks arrived after cancelled result"
    );
    println!(
        "cancel_ok chunks_before_cancel_result={chunks_at_cancel} extra_after_3s=0 stop=cancelled"
    );

    let children = child_pids(pid);
    println!("children_before_stop={children:?}");
    stop_exact_pid(&mut child);
    assert_process_gone(pid);
    for child_pid in children {
        assert_process_gone(child_pid);
        println!("child_pid={child_pid} gone=true");
    }
    println!("acp_stopped_exact_pid={pid} gone=true");
}

fn live_key() -> Option<String> {
    std::env::var("OPENROUTER_API_KEY")
        .ok()
        .filter(|value| !value.trim().is_empty())
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root")
        .to_path_buf()
}

fn send(child: &mut Child, id: u64, method: &str, params: Value) {
    let stdin = child.stdin.as_mut().expect("stdin");
    let value = json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": method,
        "params": params,
    });
    writeln!(
        stdin,
        "{}",
        serde_json::to_string(&value).expect("serialize request")
    )
    .expect("write request");
    stdin.flush().expect("flush request");
}

fn send_notification(child: &mut Child, method: &str, params: Value) {
    let stdin = child.stdin.as_mut().expect("stdin");
    let value = json!({
        "jsonrpc": "2.0",
        "method": method,
        "params": params,
    });
    writeln!(
        stdin,
        "{}",
        serde_json::to_string(&value).expect("serialize notification")
    )
    .expect("write notification");
    stdin.flush().expect("flush notification");
}

fn response_id(id: u64) -> impl FnMut(&[Value]) -> bool {
    move |messages| {
        messages
            .iter()
            .any(|message| message.get("id") == Some(&json!(id)))
    }
}

fn wait_for(
    child: &mut Child,
    rx: &mpsc::Receiver<Value>,
    messages: &mut Vec<Value>,
    mut predicate: impl FnMut(&[Value]) -> bool,
    timeout_secs: u64,
) -> Value {
    let deadline = Instant::now() + Duration::from_secs(timeout_secs);
    while Instant::now() < deadline {
        if let Some(status) = child.try_wait().expect("poll child") {
            panic!("cortex acp exited early with status {status}");
        }
        match rx.recv_timeout(Duration::from_millis(200)) {
            Ok(message) => {
                if message.get("_parse_error").is_some() {
                    panic!("stdout was not valid JSON: {message}");
                }
                messages.push(message);
                if predicate(messages) {
                    return messages.last().expect("last message").clone();
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => panic!("stdout reader disconnected"),
        }
    }
    panic!("timed out waiting for ACP message");
}

fn drain_messages(rx: &mpsc::Receiver<Value>, messages: &mut Vec<Value>) {
    while let Ok(message) = rx.try_recv() {
        messages.push(message);
    }
}

fn update_texts(messages: &[Value]) -> Vec<String> {
    messages
        .iter()
        .filter(|message| message.get("method") == Some(&json!("session/update")))
        .filter_map(|message| {
            let update = message.pointer("/params/update")?;
            (update.get("sessionUpdate") == Some(&json!("agent_message_chunk")))
                .then(|| update.pointer("/content/text"))?
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .collect()
}

fn fresh_uuid_like_nonce() -> String {
    if let Ok(output) = Command::new("uuidgen").output()
        && output.status.success()
    {
        let value = String::from_utf8_lossy(&output.stdout).trim().to_owned();
        if !value.is_empty() {
            return value;
        }
    }
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time")
        .as_nanos();
    format!("00000000-0000-4000-8000-{nanos:012x}")
}

fn assert_no_tcp_listener(pid: u32) {
    let output = Command::new("lsof")
        .args(["-Pan", "-p", &pid.to_string(), "-iTCP", "-sTCP:LISTEN"])
        .output();
    let Ok(output) = output else {
        eprintln!("lsof unavailable, skipping ACP listener check");
        return;
    };
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.trim().is_empty(),
        "ACP should own stdio and no TCP listener, got: {stdout}"
    );
    println!("tcp_listener_check=none");
}

fn child_pids(pid: u32) -> Vec<u32> {
    let output = Command::new("pgrep")
        .args(["-P", &pid.to_string()])
        .output();
    let Ok(output) = output else {
        return Vec::new();
    };
    if !matches!(output.status.code(), Some(0 | 1)) {
        return Vec::new();
    }
    String::from_utf8_lossy(&output.stdout)
        .split_whitespace()
        .filter_map(|pid| pid.parse().ok())
        .collect()
}

fn stop_exact_pid(child: &mut Child) {
    let pid = child.id();
    if child.try_wait().expect("poll child").is_some() {
        return;
    }
    let _ = Command::new("kill")
        .args(["-TERM", &pid.to_string()])
        .status();
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if child.try_wait().expect("poll child").is_some() {
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    child.kill().expect("kill exact child pid");
    child.wait().expect("wait killed child");
}

fn assert_process_gone(pid: u32) {
    let output = Command::new("ps").args(["-p", &pid.to_string()]).output();
    if let Ok(output) = output {
        assert!(!output.status.success(), "process {pid} still exists");
    }
}

struct Scratch {
    path: PathBuf,
}

impl Scratch {
    fn new(prefix: &str) -> Self {
        let unique = format!(
            "{prefix}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("time")
                .as_nanos()
        );
        let path = std::env::temp_dir().join(unique);
        std::fs::create_dir_all(&path).expect("create scratch");
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}
