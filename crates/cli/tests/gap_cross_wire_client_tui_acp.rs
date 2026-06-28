//! VAL-GAP-CROSS-004: cross-flow hardening for the compatibility surfaces.
//!
//! Proves that a wire-compatible curl client (`cortex serve`), an attached observer
//! (`cortex attach`, the same persisted-session read path the TUI's `--session` load uses), and an
//! ACP client (`cortex acp` over stdio JSON-RPC) can all operate on ONE session and converge on the
//! same identifiers and final answer:
//!
//! 1. curl creates a session and prompts it through `POST /session/{id}/message` (a real OpenRouter
//!    turn). The response carries the persisted assistant message (id + rich parts).
//! 2. curl `GET /session/{id}/message` lists the same user/assistant messages and part ids.
//! 3. `cortex attach --server <url> <id>` re-fetches the transcript through the running server and
//!    streams the same user/assistant message ids and content.
//! 4. After the server stops, `cortex acp` `session/load` replays the transcript as
//!    `session/update` notifications carrying the same `messageId`s and text.
//!
//! Evidence agreement: the session id, the user and assistant message ids, the assistant part ids
//! (server POST vs GET), and the final answer (a fresh nonce the model echoes) all match across the
//! curl client, the attach observer, and the ACP client. The real attached TUI (`cortex tui
//! --session <id>`) showing the same turn is proven live with tuistory (see the feature handoff);
//! it reads through the same `list_for_session` path this test exercises via `cortex attach`.
//!
//! The test is gated on a live OpenRouter key and skips cleanly when it is absent. The server is
//! stopped by the lsof-resolved listener PID and every spawned process group is orphan-checked.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

const RUN_TIMEOUT: Duration = Duration::from_secs(120);
const SERVER_START_TIMEOUT: Duration = Duration::from_secs(20);
const NONCE_MAX_ATTEMPTS: u32 = 3;
const NONCE_RETRY_BASE_BACKOFF_MS: u64 = 200;

fn live_key() -> Option<String> {
    match std::env::var("OPENROUTER_API_KEY") {
        Ok(key) if !key.trim().is_empty() => Some(key),
        _ => None,
    }
}

fn scratch_dir(tag: &str) -> PathBuf {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!(
        "cortex-cross-wire-{tag}-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

fn free_server_port() -> u16 {
    for port in 3240..=3299 {
        if TcpListener::bind(("127.0.0.1", port)).is_ok() {
            return port;
        }
    }
    panic!("no free server port in 3240-3299");
}

/// A cortex command sharing the live key, an isolated HOME, and the shared SQLite store so every
/// surface reads and writes the same session/message rows.
fn cortex_command(key: &str, home: &Path, db_path: &Path, cwd: &Path, args: &[&str]) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_cortex"));
    command.args(args);
    command.current_dir(cwd);
    command.env("OPENROUTER_API_KEY", key);
    command.env("HOME", home);
    command.env("CORTEX_DB_PATH", db_path);
    command
        .env_remove("CORTEX_NO_NET")
        .env_remove("RUST_LOG")
        .env_remove("CORTEX_VERBOSE")
        .env_remove("NO_COLOR");
    command
}

fn spawn_group(mut command: Command) -> Child {
    command.stdin(Stdio::null());
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());
    command.process_group(0);
    command.spawn().expect("spawn process")
}

struct ProcessResult {
    status: Option<ExitStatus>,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    pgid: u32,
}

impl ProcessResult {
    fn stdout_str(&self) -> String {
        String::from_utf8_lossy(&self.stdout).into_owned()
    }

    fn stderr_str(&self) -> String {
        String::from_utf8_lossy(&self.stderr).into_owned()
    }

    fn ok(&self) -> bool {
        self.status.and_then(|status| status.code()) == Some(0)
    }
}

fn collect_child(mut child: Child, timeout: Duration) -> ProcessResult {
    let pgid = child.id();
    let mut stdout = child.stdout.take().expect("stdout");
    let mut stderr = child.stderr.take().expect("stderr");
    let out_thread = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = stdout.read_to_end(&mut bytes);
        bytes
    });
    let err_thread = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = stderr.read_to_end(&mut bytes);
        bytes
    });
    let status = wait_for_exit(&mut child, timeout);
    ProcessResult {
        status,
        stdout: out_thread.join().expect("join stdout"),
        stderr: err_thread.join().expect("join stderr"),
        pgid,
    }
}

fn wait_for_exit(child: &mut Child, timeout: Duration) -> Option<ExitStatus> {
    let start = Instant::now();
    loop {
        if let Some(status) = child.try_wait().expect("try_wait") {
            return Some(status);
        }
        if start.elapsed() > timeout {
            let _ = child.kill();
            let _ = child.wait();
            return None;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn curl(args: &[&str]) -> ProcessResult {
    let mut command = Command::new("curl");
    command.args(args);
    collect_child(spawn_group(command), RUN_TIMEOUT)
}

fn create_session(port: u16) -> String {
    let response = curl(&[
        "-sfS",
        "-X",
        "POST",
        "-H",
        "Content-Type: application/json",
        "-d",
        "{}",
        &format!("http://127.0.0.1:{port}/session"),
    ]);
    assert!(
        response.ok(),
        "create session should succeed, stderr: {}",
        response.stderr_str()
    );
    let created: Value =
        serde_json::from_slice(&response.stdout).expect("create session returns JSON");
    created["id"]
        .as_str()
        .expect("session id present")
        .to_owned()
}

/// POST a prompt to a session and return the parsed `PromptResponse` (HTTP 200).
fn post_turn(port: u16, session: &str, text: &str) -> Value {
    let body = json!({ "text": text }).to_string();
    let response = curl(&[
        "-sfS",
        "-X",
        "POST",
        "-H",
        "Content-Type: application/json",
        "-d",
        &body,
        &format!("http://127.0.0.1:{port}/session/{session}/message"),
    ]);
    assert!(
        response.ok(),
        "server turn should succeed, stderr: {}",
        response.stderr_str()
    );
    serde_json::from_slice(&response.stdout)
        .unwrap_or_else(|err| panic!("turn JSON parse failed: {err}: {}", response.stdout_str()))
}

/// GET the full message list for a session through the running server.
fn list_messages(port: u16, session: &str) -> Vec<Value> {
    let response = curl(&[
        "-sfS",
        &format!("http://127.0.0.1:{port}/session/{session}/message"),
    ]);
    assert!(
        response.ok(),
        "message list should succeed, stderr: {}",
        response.stderr_str()
    );
    let listing: Value = serde_json::from_slice(&response.stdout).expect("message list JSON");
    listing["messages"]
        .as_array()
        .expect("messages array")
        .clone()
}

fn first_message_with_role<'a>(messages: &'a [Value], role: &str) -> &'a Value {
    messages
        .iter()
        .find(|message| message["role"] == role)
        .unwrap_or_else(|| panic!("no {role} message in the session listing"))
}

fn part_id_set(message: &Value) -> Vec<String> {
    message["parts"]
        .as_array()
        .map(|parts| {
            parts
                .iter()
                .filter_map(|part| part["id"].as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

/// Stream the session transcript through `cortex attach --server`, returning each emitted JSON
/// message line. This drives the server-attach path the way an attached observer/TUI consumes a
/// running `cortex serve`.
fn attach_via_server(
    key: &str,
    home: &Path,
    db_path: &Path,
    cwd: &Path,
    url: &str,
    session: &str,
) -> Vec<Value> {
    let result = collect_child(
        spawn_group(cortex_command(
            key,
            home,
            db_path,
            cwd,
            &[
                "attach", "--server", url, "--once", "--format", "json", session,
            ],
        )),
        RUN_TIMEOUT,
    );
    assert!(
        result.ok(),
        "attach --server should succeed, stderr: {}",
        result.stderr_str()
    );
    result
        .stdout_str()
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str::<Value>(line).expect("attach emits one JSON object/line"))
        .collect()
}

/// Drive a `cortex acp` process over stdio: write `requests` as newline-delimited JSON-RPC, close
/// stdin, and collect everything the agent wrote to stdout before EOF.
fn run_acp(key: &str, home: &Path, db_path: &Path, cwd: &Path, requests: &[Value]) -> Vec<Value> {
    let mut command = cortex_command(key, home, db_path, cwd, &["acp"]);
    command.stdin(Stdio::piped());
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());
    command.process_group(0);
    let mut child = command.spawn().expect("spawn cortex acp");
    {
        let mut stdin = child.stdin.take().expect("acp stdin");
        for request in requests {
            writeln!(
                stdin,
                "{}",
                serde_json::to_string(request).expect("serialize request")
            )
            .expect("write acp request");
        }
        // Dropping stdin sends EOF, so the agent loop drains the queued requests and exits.
    }
    let result = collect_child(child, RUN_TIMEOUT);
    assert!(
        result.ok(),
        "cortex acp should exit cleanly, stderr: {}",
        result.stderr_str()
    );
    assert_no_orphans(result.pgid);
    result
        .stdout_str()
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str::<Value>(line).expect("acp emits one JSON object/line"))
        .collect()
}

/// Extract the `(messageId, text)` of the first `session/update` of `kind` for `session`.
fn acp_chunk<'a>(messages: &'a [Value], session: &str, kind: &str) -> (&'a str, &'a str) {
    let update = messages
        .iter()
        .filter(|message| message["method"] == "session/update")
        .map(|message| &message["params"])
        .find(|params| params["sessionId"] == session && params["update"]["sessionUpdate"] == kind)
        .unwrap_or_else(|| panic!("no {kind} session/update for {session}"));
    let message_id = update["update"]["messageId"]
        .as_str()
        .unwrap_or_else(|| panic!("{kind} update should carry a messageId"));
    let text = update["update"]["content"]["text"]
        .as_str()
        .unwrap_or_else(|| panic!("{kind} update should carry text content"));
    (message_id, text)
}

fn listener_pid(port: u16) -> Option<u32> {
    let output = Command::new("lsof")
        .args(["-tiTCP", &port.to_string(), "-sTCP:LISTEN"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .find_map(|line| line.trim().parse::<u32>().ok())
}

fn signal_pid(sig: &str, pid: u32) -> bool {
    Command::new("kill")
        .args([sig, &pid.to_string()])
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

fn group_has_survivors(pgid: u32) -> Option<bool> {
    let out = Command::new("pgrep")
        .args(["-g", &pgid.to_string()])
        .output()
        .ok()?;
    match out.status.code() {
        Some(0) => Some(true),
        Some(1) => Some(false),
        _ => None,
    }
}

fn assert_no_orphans(pgid: u32) {
    if let Some(true) = group_has_survivors(pgid) {
        panic!("process group {pgid} still has live members after exit");
    }
}

fn wait_for_health(port: u16) {
    let started = Instant::now();
    loop {
        let response = curl(&["-sf", &format!("http://127.0.0.1:{port}/health")]);
        if response.ok() {
            return;
        }
        assert!(
            started.elapsed() < SERVER_START_TIMEOUT,
            "server on port {port} did not become healthy, last stderr: {}",
            response.stderr_str()
        );
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn stop_server(mut child: Child, port: u16) {
    let listener = listener_pid(port).unwrap_or_else(|| child.id());
    assert!(
        signal_pid("-TERM", listener),
        "failed to signal listener PID {listener}"
    );
    let status = wait_for_exit(&mut child, Duration::from_secs(10));
    if status.is_none() {
        let _ = signal_pid("-KILL", listener);
    }
    if let Some(mut stdout) = child.stdout.take() {
        let mut buf = Vec::new();
        let _ = stdout.read_to_end(&mut buf);
    }
    if let Some(mut stderr) = child.stderr.take() {
        let mut buf = Vec::new();
        let _ = stderr.read_to_end(&mut buf);
    }
    assert!(
        TcpListener::bind(("127.0.0.1", port)).is_ok(),
        "server port {port} should be released after stopping listener PID {listener}"
    );
}

struct ServerGuard {
    child: Option<Child>,
    port: u16,
    pgid: u32,
}

impl ServerGuard {
    fn new(child: Child, port: u16) -> Self {
        let pgid = child.id();
        Self {
            child: Some(child),
            port,
            pgid,
        }
    }

    fn stop(&mut self) {
        if let Some(child) = self.child.take() {
            stop_server(child, self.port);
        }
    }
}

impl Drop for ServerGuard {
    fn drop(&mut self) {
        self.stop();
    }
}

#[test]
fn curl_client_attach_and_acp_share_one_session_and_ids() {
    let Some(key) = live_key() else {
        eprintln!("skipping live test: OPENROUTER_API_KEY not set");
        return;
    };

    let root = scratch_dir("live");
    let home = root.join("home");
    let project = root.join("project");
    let neutral = root.join("neutral");
    std::fs::create_dir_all(&home).expect("home");
    std::fs::create_dir_all(&project).expect("project");
    std::fs::create_dir_all(&neutral).expect("neutral");
    let db_path = root.join("cortex.db");

    let port = free_server_port();
    let serve = cortex_command(
        &key,
        &home,
        &db_path,
        &project,
        &[
            "serve",
            "--hostname",
            "127.0.0.1",
            "--port",
            &port.to_string(),
        ],
    );
    let mut server = ServerGuard::new(spawn_group(serve), port);
    let server_pgid = server.pgid;
    wait_for_health(port);

    // (1) Wire-compatible curl client: create + prompt a session over the real server.
    // A live model occasionally truncates/reformats the echoed nonce, so the turn is retried on a
    // wording miss (each attempt is its own session, keeping the cross-surface id checks below bound
    // to one clean turn) and matched with a formatting-tolerant normalization. The real
    // attach/share/session-id wire-compat coverage below is unchanged.
    let (session, nonce, post) = prompt_until_nonce_echoed(port);
    assert_eq!(post["status"], "completed", "turn completed: {post}");
    let post_message = &post["message"];
    let assistant_id = post_message["id"]
        .as_str()
        .expect("POST returns the persisted assistant message id")
        .to_owned();
    let post_answer = post["content"].as_str().expect("POST answer content");
    assert!(
        echoes_nonce(post_answer, &nonce),
        "server answer should echo the nonce; got: {post_answer}"
    );
    let post_part_ids = part_id_set(post_message);
    assert!(
        !post_part_ids.is_empty(),
        "assistant message should carry rich parts with ids: {post_message}"
    );

    // (2) curl message listing: same user + assistant ids and part ids.
    let messages = list_messages(port, &session);
    let user_message = first_message_with_role(&messages, "user");
    let assistant_message = first_message_with_role(&messages, "assistant");
    let user_id = user_message["id"].as_str().expect("user id").to_owned();
    let listing_assistant_id = assistant_message["id"].as_str().expect("assistant id");
    assert_eq!(
        listing_assistant_id, assistant_id,
        "GET assistant id matches the POST assistant id"
    );
    assert_eq!(
        part_id_set(assistant_message),
        post_part_ids,
        "assistant part ids are identical between the POST response and the GET listing"
    );
    let listing_answer = assistant_message["content"]
        .as_str()
        .expect("listing answer content");
    assert!(
        echoes_nonce(listing_answer, &nonce),
        "listing answer should echo the nonce; got: {listing_answer}"
    );

    // (3) Attached observer through the running server (the TUI's persisted-session read path).
    let attached = attach_via_server(
        &key,
        &home,
        &db_path,
        &neutral,
        &format!("http://127.0.0.1:{port}"),
        &session,
    );
    let attach_user = attached
        .iter()
        .find(|message| message["role"] == "user")
        .expect("attach emits the user message");
    let attach_assistant = attached
        .iter()
        .find(|message| message["role"] == "assistant")
        .expect("attach emits the assistant message");
    assert_eq!(
        attach_user["session_id"].as_str(),
        Some(session.as_str()),
        "attach reports the same session id"
    );
    assert_eq!(
        attach_user["id"].as_str(),
        Some(user_id.as_str()),
        "attach reports the same user message id"
    );
    assert_eq!(
        attach_assistant["id"].as_str(),
        Some(assistant_id.as_str()),
        "attach reports the same assistant message id"
    );
    assert!(
        attach_assistant["content"]
            .as_str()
            .is_some_and(|content| echoes_nonce(content, &nonce)),
        "attach reports the same final answer; got: {attach_assistant}"
    );

    // Stop the server before the ACP load proves the shared store is the source of truth.
    server.stop();
    assert_no_orphans(server_pgid);

    // (4) ACP client loads the same session over stdio and replays the transcript.
    let acp_messages = run_acp(
        &key,
        &home,
        &db_path,
        &neutral,
        &[
            json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "initialize",
                "params": {
                    "protocolVersion": 1,
                    "clientCapabilities": {},
                    "clientInfo": { "name": "cross004", "version": "0" }
                }
            }),
            json!({
                "jsonrpc": "2.0",
                "id": 2,
                "method": "session/load",
                "params": { "sessionId": session }
            }),
        ],
    );
    let (acp_user_id, acp_user_text) = acp_chunk(&acp_messages, &session, "user_message_chunk");
    let (acp_assistant_id, acp_assistant_text) =
        acp_chunk(&acp_messages, &session, "agent_message_chunk");
    assert_eq!(
        acp_user_id, user_id,
        "ACP session/load replays the same user message id"
    );
    assert_eq!(
        acp_assistant_id, assistant_id,
        "ACP session/load replays the same assistant message id"
    );
    assert!(
        acp_user_text.contains(&nonce),
        "ACP user chunk carries the original prompt"
    );
    assert!(
        echoes_nonce(acp_assistant_text, &nonce),
        "ACP agent chunk carries the same final answer; got: {acp_assistant_text}"
    );
    let load_result = acp_messages
        .iter()
        .find(|message| message["id"] == 2 && message.get("result").is_some())
        .expect("session/load returns a result");
    assert_eq!(
        load_result["result"]["modes"]["currentModeId"], "build",
        "session/load result exposes the session mode state"
    );

    let _ = std::fs::remove_dir_all(&root);
}

/// A random, dependency-free token unique enough to prove the answer is freshly generated.
fn uuid_like() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{nanos:x}-{:x}", std::process::id())
}

/// Prompt a fresh session to echo a unique nonce, retrying on a live-model wording miss. Each
/// attempt uses its own session so the cross-surface id assertions stay bound to a single clean
/// turn. Returns the session id, the nonce, and the completed `PromptResponse`.
fn prompt_until_nonce_echoed(port: u16) -> (String, String, Value) {
    let mut failures = Vec::new();
    for attempt in 1..=NONCE_MAX_ATTEMPTS {
        let session = create_session(port);
        let nonce = format!("cross004-{}", uuid_like());
        let prompt = format!("Reply with exactly this token and nothing else: {nonce}");
        let post = post_turn(port, &session, &prompt);
        let completed = post["status"] == "completed";
        let answer = post["content"].as_str().unwrap_or_default();
        if completed && echoes_nonce(answer, &nonce) {
            return (session, nonce, post);
        }
        failures.push(format!(
            "attempt {attempt}/{NONCE_MAX_ATTEMPTS}, nonce={nonce:?}, status={:?}, answer={answer:?}",
            post["status"]
        ));
        if attempt < NONCE_MAX_ATTEMPTS {
            std::thread::sleep(Duration::from_millis(
                NONCE_RETRY_BASE_BACKOFF_MS * u64::from(attempt),
            ));
        }
    }
    panic!(
        "server turn failed to echo a fresh nonce after {NONCE_MAX_ATTEMPTS} attempts: {}",
        failures.join(" | ")
    );
}

/// Formatting-tolerant nonce match: compares only the ascii-alphanumeric characters (lowercased) so
/// a correctly echoed nonce still matches when the model wraps it in code fences/quotes, swaps the
/// hyphens for spaces, or changes case. Outright truncation/omission is handled by the retry above.
fn echoes_nonce(answer: &str, nonce: &str) -> bool {
    fn squeeze(text: &str) -> String {
        text.chars()
            .filter(|c| c.is_ascii_alphanumeric())
            .map(|c| c.to_ascii_lowercase())
            .collect()
    }
    squeeze(answer).contains(&squeeze(nonce))
}

/// Offline regression guard for the tolerant nonce matcher: a correctly echoed nonce must still
/// match through common live-model reformatting, while a truncated/different token must NOT match
/// (those misses are the retry's job, not the matcher's).
#[test]
fn echoes_nonce_tolerates_reformatting_but_not_truncation() {
    let nonce = "cross004-18bd5b0644f5a86d-e69a2";
    assert!(echoes_nonce(nonce, nonce), "exact echo matches");
    assert!(
        echoes_nonce("Sure, here it is: `cross004-18bd5b0644f5a86d-e69a2`", nonce),
        "code-fenced/quoted echo with surrounding prose matches"
    );
    assert!(
        echoes_nonce("cross004 18bd5b0644f5a86d e69a2.", nonce),
        "hyphens reflowed to spaces with trailing punctuation matches"
    );
    assert!(
        echoes_nonce("CROSS004-18BD5B0644F5A86D-E69A2", nonce),
        "case-changed echo matches"
    );
    assert!(
        !echoes_nonce("cross004-18bd5b0644f5a86d", nonce),
        "a truncated nonce must not match (handled by retry)"
    );
    assert!(
        !echoes_nonce("totally different answer", nonce),
        "an unrelated answer must not match"
    );
}
