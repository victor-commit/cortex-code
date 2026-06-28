//! VAL-GAP-CROSS-003: cross-flow hardening for compaction.
//!
//! Proves that a long server-driven session and the headless resume share ONE consistent compacted
//! transcript by composing five subsystems end-to-end against a real `cortex serve` process:
//!
//! 1. A long server session (driven over `POST /session/{id}/message` with a low
//!    `CORTEX_CONTEXT_LIMIT_TOKENS`) compacts its history through a real, model-authored summary.
//! 2. A configured local plugin's `experimental.session.compacting` and
//!    `experimental.compaction.autocontinue` hooks fire on the server turn, altering the persisted
//!    summary metadata (the server now applies plugin compaction hooks, mirroring headless `run`).
//! 3. The SSE `/event` stream carries the `session.compacted` transition.
//! 4. The persisted session state (server `GET /session/{id}/message` AND headless `cortex export`)
//!    exposes the same compaction part with the plugin-transformed summary.
//! 5. After the server stops, a headless `cortex run --session <id>` resume answers an old fact from
//!    the compacted history.
//!
//! Evidence agreement: the SSE event's `tail_start_id`, the plugin hook log, the server/headless
//! export compaction part, and the final answer all describe the same compaction. The test is gated
//! on a live OpenRouter key AND a JS runtime; it skips cleanly when either is absent. The server is
//! stopped by the lsof-resolved listener PID and orphan-checked.

use std::io::Read;
use std::net::TcpListener;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use serde_json::Value;

const RUN_TIMEOUT: Duration = Duration::from_secs(120);
const SERVER_START_TIMEOUT: Duration = Duration::from_secs(20);
/// A low effective context window so compaction provably engages on a modest server session while
/// keeping each turn cheap. The runner only emits a model-authored compaction record once the
/// session has more than `DEFAULT_TAIL_MESSAGES` (8) user turns, so the loop drives several turns.
const CONTEXT_LIMIT_TOKENS: &str = "1500";
/// The fact seeded early and recalled from the compacted history after resume.
const CODENAME: &str = "BLUEBIRD";
/// A marker the plugin appends to every compaction summary it transforms.
const PLUGIN_MARK: &str = "[PLUGIN_COMPACTED]";

fn live_key() -> Option<String> {
    match std::env::var("OPENROUTER_API_KEY") {
        Ok(key) if !key.trim().is_empty() => Some(key),
        _ => None,
    }
}

fn scratch_dir(tag: &str) -> PathBuf {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!(
        "cortex-cross-compaction-{tag}-{}-{}",
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

/// A cortex command sharing the live key, isolated HOME, and the shared SQLite store. `cwd` governs
/// which config layer (and therefore which plugins) is discovered.
fn cortex_command(key: &str, home: &Path, db_path: &Path, cwd: &Path, args: &[&str]) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_cortex"));
    command.args(args);
    command.current_dir(cwd);
    command.env("OPENROUTER_API_KEY", key);
    command.env("HOME", home);
    command.env("CORTEX_DB_PATH", db_path);
    command
        .env_remove("CORTEX_NO_NET")
        .env_remove("CORTEX_CONTEXT_LIMIT_TOKENS")
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

/// POST a prompt to a session and assert the turn completed (HTTP 200, JSON body).
fn post_turn(port: u16, session: &str, text: &str) -> Value {
    let body = serde_json::json!({ "text": text }).to_string();
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
}

impl Drop for ServerGuard {
    fn drop(&mut self) {
        if let Some(child) = self.child.take() {
            stop_server(child, self.port);
        }
    }
}

/// Parse the captured SSE log for the first `session.compacted` event for `session`.
fn sse_compacted_event(sse_log: &Path, session: &str) -> Option<Value> {
    let text = std::fs::read_to_string(sse_log).ok()?;
    text.lines()
        .filter_map(|line| line.strip_prefix("data:"))
        .filter_map(|payload| serde_json::from_str::<Value>(payload.trim()).ok())
        .find(|event| event["kind"] == "session.compacted" && event["session_id"] == session)
}

/// Find the first compaction part across a message list's `parts` arrays.
fn first_compaction_part(messages: &[Value]) -> Option<Value> {
    messages.iter().find_map(|message| {
        message["parts"]
            .as_array()?
            .iter()
            .find(|part| part["type"] == "compaction")
            .cloned()
    })
}

fn write_compaction_plugin(project: &Path, marker: &Path) {
    let plugin = project.join("plugin.mjs");
    std::fs::write(
        &plugin,
        r#"
import { appendFile } from "node:fs/promises";

export default async function plugin(input, options) {
  async function mark(payload) {
    await appendFile(options.marker, JSON.stringify(payload) + "\n");
  }
  return {
    experimental: {
      session: {
        async compacting(record) {
          await mark({
            hook: "experimental.session.compacting",
            tail_start_id: record.tail_start_id ?? null,
            model: record.model ?? null
          });
          // Augment (do not replace) the model-authored summary so the preserved facts survive and
          // the plugin transformation is observable in the persisted compaction part.
          return { summary: `${record.summary ?? ""} [PLUGIN_COMPACTED]` };
        }
      },
      compaction: {
        async autocontinue(record) {
          await mark({
            hook: "experimental.compaction.autocontinue",
            tail_start_id: record.tail_start_id ?? null
          });
          return { continued_request: true };
        }
      }
    }
  };
}
"#,
    )
    .expect("write plugin");
    std::fs::write(
        project.join("cortex.toml"),
        format!(
            "plugin = [[\"./plugin.mjs\", {{ marker = \"{}\" }}]]\n",
            marker.display()
        ),
    )
    .expect("write config");
}

#[test]
fn server_compaction_plugin_sse_and_headless_resume_share_one_transcript() {
    let Some(key) = live_key() else {
        eprintln!("skipping live test: OPENROUTER_API_KEY not set");
        return;
    };
    if plugins::detect_runtime().is_none() {
        eprintln!("skipping cross compaction test: no Bun, Node, or Deno on PATH");
        return;
    }

    let root = scratch_dir("live");
    let home = root.join("home");
    let project = root.join("project");
    let neutral = root.join("neutral");
    std::fs::create_dir_all(&home).expect("home");
    std::fs::create_dir_all(&project).expect("project");
    std::fs::create_dir_all(&neutral).expect("neutral");
    let db_path = root.join("cortex.db");
    let marker = project.join("hooks.jsonl");
    let audit_log = project.join("audit.jsonl");
    write_compaction_plugin(&project, &marker);

    let port = free_server_port();
    let mut serve = cortex_command(
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
    serve.env("CORTEX_CONTEXT_LIMIT_TOKENS", CONTEXT_LIMIT_TOKENS);
    serve.env("CORTEX_AUDIT_LOG", &audit_log);
    let server = ServerGuard::new(spawn_group(serve), port);
    let server_pgid = server.pgid;
    wait_for_health(port);

    // Background SSE capture of the typed event stream.
    let sse_log_path = root.join("events.sse");
    let sse_file = std::fs::File::create(&sse_log_path).expect("create sse log");
    let mut sse_command = Command::new("curl");
    sse_command.args(["-sN", &format!("http://127.0.0.1:{port}/event")]);
    sse_command.stdin(Stdio::null());
    sse_command.stdout(Stdio::from(sse_file));
    sse_command.stderr(Stdio::null());
    sse_command.process_group(0);
    let mut sse_child = sse_command.spawn().expect("spawn sse capture");
    let sse_pgid = sse_child.id();

    // Create the session.
    let create = curl(&[
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
        create.ok(),
        "create session stderr: {}",
        create.stderr_str()
    );
    let created: Value = serde_json::from_slice(&create.stdout).expect("session JSON");
    let session = created["id"].as_str().expect("session id").to_owned();

    // Seed the fact, then drive growth turns until a model-authored compaction is observed on SSE.
    post_turn(
        port,
        &session,
        &format!(
            "Remember this codename and preserve it in any summary you ever write: {CODENAME}. \
             Reply with just OK."
        ),
    );
    let filler = "context padding detail ".repeat(40);
    let mut compacted = None;
    for turn in 0..14 {
        post_turn(
            port,
            &session,
            &format!("{filler}\nTurn {turn}: acknowledge with one short word."),
        );
        // Give the SSE frame a moment to flush, then check for the compaction transition.
        std::thread::sleep(Duration::from_millis(300));
        if let Some(event) = sse_compacted_event(&sse_log_path, &session) {
            compacted = Some(event);
            break;
        }
    }

    let sse_event = compacted.unwrap_or_else(|| {
        panic!(
            "no session.compacted SSE event observed after driving the session; sse log: {}",
            std::fs::read_to_string(&sse_log_path).unwrap_or_default()
        )
    });
    let sse_tail = sse_event["tail_start_id"]
        .as_str()
        .expect("session.compacted carries tail_start_id")
        .to_owned();
    assert!(
        sse_tail.starts_with("tail_"),
        "tail_start_id should be a stable tail token: {sse_tail}"
    );

    // The plugin compaction hooks fired on the server turn (plugin log evidence).
    let hook_log = std::fs::read_to_string(&marker).expect("plugin hook log exists");
    assert!(
        hook_log.contains("experimental.session.compacting"),
        "session.compacting hook should fire on the server compaction: {hook_log}"
    );
    assert!(
        hook_log.contains("experimental.compaction.autocontinue"),
        "compaction.autocontinue hook should fire on the server compaction: {hook_log}"
    );
    let hook_entry = hook_log
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .find(|entry| entry["hook"] == "experimental.session.compacting")
        .expect("a session.compacting marker entry");
    assert_eq!(
        hook_entry["tail_start_id"], sse_tail,
        "plugin hook saw the same compaction tail as the SSE event: {hook_entry}"
    );

    // The audit log records the server-attributed plugin compaction-hook invocations.
    let audit_records = audit::read_jsonl(&audit_log).expect("audit log parseable");
    assert!(
        audit_records.iter().any(|record| {
            record["type"] == "plugin_hook"
                && record["hook"] == "experimental.session.compacting"
                && record["action"] == "transform_compaction"
                && record["source"] == "server"
        }),
        "server audit log should record the compaction hook: {audit_records:#?}"
    );

    // Server session state export: the persisted compaction part carries the plugin-transformed
    // summary and agrees with the SSE tail.
    let listing = curl(&[
        "-sfS",
        &format!("http://127.0.0.1:{port}/session/{session}/message"),
    ]);
    assert!(
        listing.ok(),
        "message list stderr: {}",
        listing.stderr_str()
    );
    let listing: Value = serde_json::from_slice(&listing.stdout).expect("message list JSON");
    let server_messages = listing["messages"].as_array().expect("messages array");
    let server_part = first_compaction_part(server_messages)
        .expect("server export should expose a compaction part");
    assert_eq!(
        server_part["tail_start_id"], sse_tail,
        "server compaction part tail agrees with the SSE event: {server_part}"
    );
    let server_summary = server_part["summary"].as_str().unwrap_or_default();
    assert!(
        server_summary.contains(PLUGIN_MARK),
        "server compaction summary should carry the plugin transformation marker: {server_part}"
    );

    // Tear down the SSE capture and the server before the headless resume.
    let _ = signal_pid("-TERM", sse_pgid);
    let _ = sse_child.wait();
    drop(server);
    assert_no_orphans(server_pgid);
    assert_no_orphans(sse_pgid);

    // Headless resume from a neutral cwd (no plugin config) over the SAME store: the compacted
    // history still answers the old fact.
    let recall = collect_child(
        spawn_group(cortex_command(
            &key,
            &home,
            &db_path,
            &neutral,
            &[
                "run",
                "--session",
                &session,
                "What codename did I ask you to remember earlier? Reply with only the codename.",
            ],
        )),
        RUN_TIMEOUT,
    );
    assert!(
        recall.ok(),
        "headless resume should succeed, stderr: {}",
        recall.stderr_str()
    );
    assert!(
        recall.stdout_str().contains(CODENAME),
        "headless resume should recall the fact from the compacted history; got: {}",
        recall.stdout_str()
    );
    assert_no_orphans(recall.pgid);

    // Headless export agrees with the server export: same compaction part (tail + transformed
    // summary), proving one shared compacted transcript across surfaces.
    let exported = collect_child(
        spawn_group(cortex_command(
            &key,
            &home,
            &db_path,
            &neutral,
            &["export", &session],
        )),
        RUN_TIMEOUT,
    );
    assert!(
        exported.ok(),
        "headless export should succeed, stderr: {}",
        exported.stderr_str()
    );
    let artifact: Value = serde_json::from_slice(&exported.stdout).expect("export JSON");
    let export_messages = artifact["messages"].as_array().expect("export messages");
    let export_part = first_compaction_part(export_messages)
        .expect("headless export should expose the same compaction part");
    assert_eq!(
        export_part["tail_start_id"], sse_tail,
        "headless export compaction tail agrees with the SSE event and server export"
    );
    assert!(
        export_part["summary"]
            .as_str()
            .is_some_and(|summary| summary.contains(PLUGIN_MARK)),
        "headless export compaction summary carries the plugin transformation: {export_part}"
    );
    assert_no_orphans(exported.pgid);

    let _ = std::fs::remove_dir_all(&root);
}
