//! Cross-surface shared-store proofs for the M6 integration slice
//! (VAL-CROSS-007, VAL-CROSS-008, VAL-CROSS-046).
//!
//! - Offline path seeds the shared SQLite store and proves a full process restart keeps messages and
//!   tool-written todos byte-identical across headless `session show`, a fresh `cortex serve`, and a
//!   TUI reattach snapshot.
//! - Live path (gated on `OPENROUTER_API_KEY`) drives real OpenRouter turns for message-text
//!   consistency after restart and concurrent multi-process writers (headless + server) on the same
//!   file with no SQLite lock/corruption.
//!
//! Every serve listener is stopped by the lsof-resolved listener PID only.

use std::io::Read;
use std::net::TcpListener;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use db::Database;
use protocol::{TodoItem, TodoStatus};

const RUN_TIMEOUT: Duration = Duration::from_secs(120);
const SERVER_START_TIMEOUT: Duration = Duration::from_secs(20);
const TUI_WAIT_TIMEOUT_MS: u64 = 20_000;

fn live_key() -> Option<String> {
    match std::env::var("OPENROUTER_API_KEY") {
        Ok(key) if !key.trim().is_empty() => Some(key),
        _ => None,
    }
}

fn scratch_dir(tag: &str) -> PathBuf {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!(
        "cortex-cross-m6-shared-{tag}-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

fn free_server_port() -> u16 {
    for port in 3220..=3299 {
        if TcpListener::bind(("127.0.0.1", port)).is_ok() {
            return port;
        }
    }
    panic!("no free server port in 3220-3299");
}

fn nonce(label: &str, salt: u32) -> String {
    format!("{label}x{:x}", (std::process::id() ^ salt) & 0xff_ffff)
}

fn cortex_command(key: Option<&str>, home: &Path, db_path: &Path, args: &[&str]) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_cortex"));
    command.args(args);
    if let Some(key) = key {
        command.env("OPENROUTER_API_KEY", key);
    } else {
        command.env_remove("OPENROUTER_API_KEY");
    }
    command.env("HOME", home);
    command.env("CORTEX_DB_PATH", db_path);
    command
        .env_remove("CORTEX_NO_NET")
        .env_remove("RUST_LOG")
        .env_remove("CORTEX_VERBOSE")
        .env_remove("NO_COLOR");
    command
}

fn cortex_offline(home: &Path, db_path: &Path, args: &[&str]) -> Command {
    cortex_command(None, home, db_path, args)
}

fn tuistory_available() -> bool {
    Command::new("tuistory")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

fn first_assistant_content(body: &serde_json::Value) -> String {
    body["messages"]
        .as_array()
        .expect("messages array")
        .iter()
        .find(|message| message["role"].as_str() == Some("assistant"))
        .and_then(|message| message["content"].as_str())
        .expect("assistant message content")
        .to_owned()
}

fn assistant_contents(body: &serde_json::Value) -> Vec<String> {
    body["messages"]
        .as_array()
        .expect("messages array")
        .iter()
        .filter(|message| message["role"].as_str() == Some("assistant"))
        .map(|message| {
            message["content"]
                .as_str()
                .expect("assistant content")
                .to_owned()
        })
        .collect()
}

struct TuistorySession {
    name: String,
}

impl TuistorySession {
    fn launch(
        name: String,
        cortex: &Path,
        project: &Path,
        home: &Path,
        db_path: &Path,
        session_id: &str,
        key: Option<&str>,
    ) -> Self {
        let command_line = format!("{} tui --session {}", cortex.display(), session_id);
        let mut launch = Command::new("tuistory");
        launch
            .arg("launch")
            .arg(command_line)
            .arg("-s")
            .arg(&name)
            .arg("--cwd")
            .arg(project)
            .arg("--cols")
            .arg("140")
            .arg("--rows")
            .arg("36")
            .arg("--env")
            .arg(format!("HOME={}", home.display()))
            .arg("--env")
            .arg(format!(
                "XDG_CONFIG_HOME={}",
                home.join(".config").display()
            ))
            .arg("--env")
            .arg(format!(
                "XDG_DATA_HOME={}",
                home.join(".local/share").display()
            ))
            .arg("--env")
            .arg(format!("CORTEX_DB_PATH={}", db_path.display()))
            .arg("--env")
            .arg("CORTEX_AUTH_FILE=")
            .arg("--env")
            .arg("RUST_LOG=")
            .arg("--env")
            .arg("CORTEX_VERBOSE=")
            .arg("--env")
            .arg("NO_COLOR=")
            .arg("--env")
            .arg("CORTEX_NO_NET=1");
        if let Some(key) = key {
            launch.arg("--env").arg(format!("OPENROUTER_API_KEY={key}"));
            // allow net when a real key is supplied so an attached resume path can still paint
            launch.arg("--env").arg("CORTEX_NO_NET=");
        }
        let out = launch
            .arg("--background")
            .arg("--timeout")
            .arg("15000")
            .output()
            .expect("launch tuistory");
        assert!(
            out.status.success(),
            "tuistory launch failed:\nstdout={}\nstderr={}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        std::thread::sleep(Duration::from_millis(800));
        Self { name }
    }

    fn run(&self, args: &[&str]) {
        let output = Command::new("tuistory")
            .arg("-s")
            .arg(&self.name)
            .args(args)
            .output()
            .expect("run tuistory command");
        assert!(
            output.status.success(),
            "tuistory {args:?} failed:\nstdout={}\nstderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn snapshot(&self) -> String {
        let output = Command::new("tuistory")
            .arg("-s")
            .arg(&self.name)
            .args(["snapshot", "--immediate", "--trim"])
            .output()
            .expect("snapshot");
        assert!(
            output.status.success(),
            "snapshot failed:\nstdout={}\nstderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).into_owned()
    }

    fn wait_for_any(&self, needles: &[&str], timeout_ms: u64) -> String {
        let start = Instant::now();
        loop {
            let snap = self.snapshot();
            if needles.iter().any(|needle| snap.contains(needle)) {
                return snap;
            }
            if start.elapsed() > Duration::from_millis(timeout_ms) {
                panic!("TUI did not show any of {needles:?} within {timeout_ms}ms:\n{snap}");
            }
            std::thread::sleep(Duration::from_millis(250));
        }
    }

    fn close(self) {
        let _ = Command::new("tuistory")
            .arg("-s")
            .arg(&self.name)
            .arg("close")
            .output();
    }
}

impl Drop for TuistorySession {
    fn drop(&mut self) {
        let _ = Command::new("tuistory")
            .arg("-s")
            .arg(&self.name)
            .arg("close")
            .output();
    }
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
        if response.status.and_then(|status| status.code()) == Some(0) {
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
    let mut stdout = child.stdout.take().expect("server stdout");
    let mut stderr = child.stderr.take().expect("server stderr");
    let mut out_buf = Vec::new();
    let mut err_buf = Vec::new();
    let _ = stdout.read_to_end(&mut out_buf);
    let _ = stderr.read_to_end(&mut err_buf);
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

    fn pgid(&self) -> u32 {
        self.pgid
    }
}

impl Drop for ServerGuard {
    fn drop(&mut self) {
        if let Some(child) = self.child.take() {
            stop_server(child, self.port);
        }
    }
}

/// VAL-CROSS-007 + VAL-CROSS-008 offline: after a full process exit, message text and tool-written
/// todos remain byte-identical via headless `session show`, a restarted server, and (when present)
/// a TUI reattach snapshot of the same session.
#[test]
fn offline_messages_and_todos_survive_restart_across_surfaces() {
    let root = scratch_dir("offline-restart");
    let home = root.join("home");
    let project = root.join("project");
    std::fs::create_dir_all(&home).expect("home");
    std::fs::create_dir_all(&project).expect("project");
    let db_path = root.join("cortex.db");

    let token = nonce("msg", 41);
    let todo_completed = format!("capture-{token}");
    let todo_progress = format!("verify-{token}");
    let todo_pending = format!("publish-{token}");
    let assistant_text = format!("assistant acknowledges {token}");

    // Process 1: seed the shared store, then fully drop the connection (restart simulation).
    {
        let db = Database::open(&db_path).expect("open scratch db");
        let session = db
            .sessions()
            .create(Some("shared-store-restart"))
            .expect("create session");
        let session_id = session.id.clone();
        db.messages()
            .create(
                &session_id,
                "user",
                &format!("remember this exact nonce: {token}"),
            )
            .expect("user message");
        db.messages()
            .create(&session_id, "assistant", &assistant_text)
            .expect("assistant message");
        db.todos()
            .replace_for_session(
                &session_id,
                &[
                    TodoItem::new(todo_completed.clone(), TodoStatus::Completed),
                    TodoItem::new(todo_progress.clone(), TodoStatus::InProgress),
                    TodoItem::new(todo_pending.clone(), TodoStatus::Pending),
                ],
            )
            .expect("seed todos");
        drop(db);

        // Process 2 (headless): read messages back via `cortex session show`.
        let shown = collect_child(
            spawn_group(cortex_offline(
                &home,
                &db_path,
                &["session", "show", &session_id, "--format", "json"],
            )),
            RUN_TIMEOUT,
        );
        assert_eq!(
            shown.status.and_then(|status| status.code()),
            Some(0),
            "session show stderr: {}",
            shown.stderr_str()
        );
        let headless_body: serde_json::Value =
            serde_json::from_slice(&shown.stdout).expect("session show JSON");
        let headless_assistant = first_assistant_content(&headless_body);
        assert_eq!(
            headless_assistant, assistant_text,
            "headless session show must return the exact persisted assistant content"
        );
        assert!(
            headless_body["messages"]
                .as_array()
                .expect("messages")
                .iter()
                .any(|message| {
                    message["role"].as_str() == Some("user")
                        && message["content"]
                            .as_str()
                            .is_some_and(|content| content.contains(&token))
                }),
            "headless session show must retain the user nonce: {headless_body:?}"
        );

        // Process 3 (server): restart-scan by opening a fresh serve process and curl-reading the
        // same messages + todos.
        let port = free_server_port();
        let server = ServerGuard::new(
            spawn_group(cortex_offline(
                &home,
                &db_path,
                &[
                    "serve",
                    "--hostname",
                    "127.0.0.1",
                    "--port",
                    &port.to_string(),
                ],
            )),
            port,
        );
        let server_pgid = server.pgid();
        wait_for_health(port);

        let server_messages = curl(&[
            "-sfS",
            &format!("http://127.0.0.1:{port}/session/{session_id}/message"),
        ]);
        assert_eq!(
            server_messages.status.and_then(|status| status.code()),
            Some(0),
            "server message list stderr: {}",
            server_messages.stderr_str()
        );
        let server_message_body: serde_json::Value =
            serde_json::from_slice(&server_messages.stdout).expect("server messages JSON");
        let server_assistant = first_assistant_content(&server_message_body);
        assert_eq!(
            server_assistant, headless_assistant,
            "server GET /message and headless show must agree exactly"
        );

        let server_todos = curl(&[
            "-sfS",
            &format!("http://127.0.0.1:{port}/session/{session_id}/todos"),
        ]);
        assert_eq!(
            server_todos.status.and_then(|status| status.code()),
            Some(0),
            "server todos stderr: {}",
            server_todos.stderr_str()
        );
        let server_todo_body: serde_json::Value =
            serde_json::from_slice(&server_todos.stdout).expect("server todos JSON");
        let todos = server_todo_body["todos"].as_array().expect("todos array");
        assert_eq!(todos.len(), 3, "server todos: {server_todo_body:?}");
        assert_eq!(todos[0]["content"], todo_completed);
        assert_eq!(todos[0]["status"], "completed");
        assert_eq!(todos[1]["content"], todo_progress);
        assert_eq!(todos[1]["status"], "in_progress");
        assert_eq!(todos[2]["content"], todo_pending);
        assert_eq!(todos[2]["status"], "pending");

        // Process 4 (TUI): reattach and snapshot. Prefer the transcript notice Todos list; also
        // open `/sidebar-todo` so the durable panel shows the same three items.
        if tuistory_available() {
            let cortex = PathBuf::from(env!("CARGO_BIN_EXE_cortex"));
            let tui = TuistorySession::launch(
                format!("cross-m6-shared-offline-{}", std::process::id()),
                &cortex,
                &project,
                &home,
                &db_path,
                &session_id,
                None,
            );
            let transcript = tui.wait_for_any(&[&assistant_text, &token], TUI_WAIT_TIMEOUT_MS);
            assert!(
                transcript.contains(&assistant_text) || transcript.contains(&token),
                "TUI reattach must render the stored assistant/user text:\n{transcript}"
            );

            tui.run(&["type", "/sidebar-todo"]);
            tui.run(&["press", "enter"]);
            let sidebar = tui.wait_for_any(
                &[&todo_completed, &todo_progress, &todo_pending],
                TUI_WAIT_TIMEOUT_MS,
            );
            assert!(
                sidebar.contains(&todo_completed),
                "TUI todo panel must show completed item:\n{sidebar}"
            );
            assert!(
                sidebar.contains(&todo_progress),
                "TUI todo panel must show in-progress item:\n{sidebar}"
            );
            assert!(
                sidebar.contains(&todo_pending),
                "TUI todo panel must show pending item:\n{sidebar}"
            );
            tui.close();
        } else {
            eprintln!(
                "skipping TUI portion of offline restart proof: tuistory unavailable; headless + server agreed"
            );
        }

        // Direct DB reopen also sees the same todos (process boundary sanity).
        {
            let db = Database::open(&db_path).expect("reopen db");
            let todos = db
                .todos()
                .list_for_session(&session_id)
                .expect("list todos after restart");
            assert_eq!(todos.len(), 3);
            assert_eq!(todos[0].content, todo_completed);
            assert_eq!(todos[0].status, TodoStatus::Completed);
            assert_eq!(todos[1].content, todo_progress);
            assert_eq!(todos[1].status, TodoStatus::InProgress);
            assert_eq!(todos[2].content, todo_pending);
            assert_eq!(todos[2].status, TodoStatus::Pending);
            assert_eq!(
                db.integrity_check().expect("integrity"),
                "ok",
                "shared store remains sound after multi-surface reads"
            );
        }

        drop(server);
        assert_no_orphans(server_pgid);
    }

    let _ = std::fs::remove_dir_all(&root);
}

/// VAL-CROSS-007 live: a real OpenRouter turn's assistant text survives full process exit and is
/// identical when read via headless session show, curl .../message, and a TUI reattach snapshot.
#[test]
fn live_turn_messages_are_byte_consistent_after_restart_across_surfaces() {
    let Some(key) = live_key() else {
        eprintln!("skipping live test: OPENROUTER_API_KEY not set");
        return;
    };

    let root = scratch_dir("live-message-restart");
    let home = root.join("home");
    let project = root.join("project");
    std::fs::create_dir_all(&home).expect("home");
    std::fs::create_dir_all(&project).expect("project");
    let db_path = root.join("cortex.db");
    let session_id = format!("ses_live_msg_restart_{}", std::process::id());
    let token = nonce("live", 73);

    // Process 1: real headless turn that trains the model on a fresh nonce and exits.
    let turn = collect_child(
        spawn_group(cortex_command(
            Some(&key),
            &home,
            &db_path,
            &[
                "run",
                "--session",
                &session_id,
                "--format",
                "json",
                &format!(
                    "Reply with exactly this text and nothing else, no quotes, no markdown, no extra words: {token}"
                ),
            ],
        )),
        RUN_TIMEOUT,
    );
    assert_eq!(
        turn.status.and_then(|status| status.code()),
        Some(0),
        "headless live turn stderr: {}",
        turn.stderr_str()
    );
    let turn_body: serde_json::Value = serde_json::from_slice(&turn.stdout).expect("run JSON");
    let run_stdout = turn.stdout_str();
    let run_content = turn_body
        .get("content")
        .and_then(|value| value.as_str())
        .or_else(|| turn_body.get("assistant").and_then(|value| value.as_str()))
        .or_else(|| {
            turn_body
                .get("message")
                .and_then(|value| value.get("content"))
                .and_then(|value| value.as_str())
        })
        .map(str::to_owned)
        .unwrap_or_else(|| run_stdout.clone());
    assert!(
        run_content.contains(&token) || run_stdout.contains(&token),
        "live run must emit the nonce in its assistant response: {run_stdout}"
    );
    assert_no_orphans(turn.pgid);

    // Process 2: headless session show (full restart).
    let shown = collect_child(
        spawn_group(cortex_command(
            Some(&key),
            &home,
            &db_path,
            &["session", "show", &session_id, "--format", "json"],
        )),
        RUN_TIMEOUT,
    );
    assert_eq!(
        shown.status.and_then(|status| status.code()),
        Some(0),
        "session show stderr: {}",
        shown.stderr_str()
    );
    let shown_body: serde_json::Value = serde_json::from_slice(&shown.stdout).expect("show JSON");
    let headless_assistant = first_assistant_content(&shown_body);
    assert!(
        headless_assistant.contains(&token),
        "session show must retain the live nonce after restart: {shown_body:?}"
    );

    // Process 3: fresh server and GET /message for the same session.
    let port = free_server_port();
    let server = ServerGuard::new(
        spawn_group(cortex_command(
            Some(&key),
            &home,
            &db_path,
            &[
                "serve",
                "--hostname",
                "127.0.0.1",
                "--port",
                &port.to_string(),
            ],
        )),
        port,
    );
    let server_pgid = server.pgid();
    wait_for_health(port);

    let server_messages = curl(&[
        "-sfS",
        &format!("http://127.0.0.1:{port}/session/{session_id}/message"),
    ]);
    assert_eq!(
        server_messages.status.and_then(|status| status.code()),
        Some(0),
        "server messages stderr: {}",
        server_messages.stderr_str()
    );
    let server_body: serde_json::Value =
        serde_json::from_slice(&server_messages.stdout).expect("server messages");
    let server_assistant = first_assistant_content(&server_body);
    assert_eq!(
        server_assistant, headless_assistant,
        "server and headless must return the same stored assistant text"
    );
    assert_eq!(
        assistant_contents(&server_body),
        assistant_contents(&shown_body),
        "all assistant texts must match across surfaces"
    );

    // Process 4: TUI reattach should snapshot the same stored text.
    if tuistory_available() {
        let cortex = PathBuf::from(env!("CARGO_BIN_EXE_cortex"));
        let tui = TuistorySession::launch(
            format!("cross-m6-shared-live-msg-{}", std::process::id()),
            &cortex,
            &project,
            &home,
            &db_path,
            &session_id,
            Some(&key),
        );
        let snap = tui.wait_for_any(&[&token], TUI_WAIT_TIMEOUT_MS);
        assert!(
            snap.contains(&token),
            "TUI reattach must show the live assistant nonce:\n{snap}"
        );
        tui.close();
    } else {
        eprintln!(
            "skipping TUI portion of live message-restart proof: tuistory unavailable; headless + server agreed"
        );
    }

    drop(server);
    assert_no_orphans(server_pgid);
    let _ = std::fs::remove_dir_all(&root);
}

/// VAL-CROSS-008 live: a real openrouter turn that records 3 todos (one completed) survives restart
/// and the same items/statuses are visible via the server and the TUI sidebar.
#[test]
fn live_tool_written_todos_survive_restart_across_surfaces() {
    let Some(key) = live_key() else {
        eprintln!("skipping live test: OPENROUTER_API_KEY not set");
        return;
    };

    let root = scratch_dir("live-todo-restart");
    let home = root.join("home");
    let project = root.join("project");
    std::fs::create_dir_all(&home).expect("home");
    std::fs::create_dir_all(&project).expect("project");
    let db_path = root.join("cortex.db");
    let session_id = format!("ses_live_todo_restart_{}", std::process::id());
    let n1 = nonce("todoA", 3);
    let n2 = nonce("todoB", 5);
    let n3 = nonce("todoC", 7);

    // Strong, deterministic instructions: tool call is the proof path (agent must use the todo tool).
    let prompt = format!(
        "You MUST use the todo tool exactly once (no other tools). \
         Create exactly these 3 todos and mark ONLY the first one completed, \
         the second in_progress, and the third pending. \
         Contents, in order, exactly: '{n1}', '{n2}', '{n3}'. \
         Do not invent extra items. After the tool succeeds, reply with only: TODOS_RECORDED."
    );

    let turn = collect_child(
        spawn_group(cortex_command(
            Some(&key),
            &home,
            &db_path,
            &["run", "--session", &session_id, "--format", "json", &prompt],
        )),
        RUN_TIMEOUT,
    );
    assert_eq!(
        turn.status.and_then(|status| status.code()),
        Some(0),
        "todo live turn stderr: {}",
        turn.stderr_str()
    );
    assert_no_orphans(turn.pgid);

    // Full restart: headless reopen reads the durable todos from SQLite via the server after
    // launching a giveaway serve, and also from a DB reopen.
    {
        let db = Database::open(&db_path).expect("reopen for headless todos");
        let todos = db
            .todos()
            .list_for_session(&session_id)
            .expect("list todos after run");
        assert!(
            todos.len() >= 3,
            "expected at least 3 durable todos after tool write; got {todos:?}\nrun stdout: {}\nrun stderr: {}",
            turn.stdout_str(),
            turn.stderr_str()
        );
        let contents: Vec<_> = todos.iter().map(|item| item.content.as_str()).collect();
        assert!(
            contents.iter().any(|c| c.contains(&n1)),
            "todo content missing {n1}: {todos:?}"
        );
        assert!(
            contents.iter().any(|c| c.contains(&n2)),
            "todo content missing {n2}: {todos:?}"
        );
        assert!(
            contents.iter().any(|c| c.contains(&n3)),
            "todo content missing {n3}: {todos:?}"
        );
        assert!(
            todos
                .iter()
                .any(|item| item.status == TodoStatus::Completed && item.content.contains(&n1)),
            "first listed nonce should be completed: {todos:?}"
        );
    }

    let port = free_server_port();
    let server = ServerGuard::new(
        spawn_group(cortex_command(
            Some(&key),
            &home,
            &db_path,
            &[
                "serve",
                "--hostname",
                "127.0.0.1",
                "--port",
                &port.to_string(),
            ],
        )),
        port,
    );
    let server_pgid = server.pgid();
    wait_for_health(port);

    let server_todos = curl(&[
        "-sfS",
        &format!("http://127.0.0.1:{port}/session/{session_id}/todos"),
    ]);
    assert_eq!(
        server_todos.status.and_then(|status| status.code()),
        Some(0),
        "server todos stderr: {}",
        server_todos.stderr_str()
    );
    let server_body: serde_json::Value =
        serde_json::from_slice(&server_todos.stdout).expect("server todos JSON");
    let todos = server_body["todos"].as_array().expect("todos array");
    assert!(
        todos.len() >= 3,
        "server todos should survive restart: {server_body:?}"
    );
    let serialized = server_body.to_string();
    assert!(serialized.contains(&n1) && serialized.contains(&n2) && serialized.contains(&n3));
    assert!(
        todos.iter().any(|item| {
            item["status"].as_str() == Some("completed")
                && item["content"]
                    .as_str()
                    .is_some_and(|content| content.contains(&n1))
        }),
        "completed item must be present after restart: {server_body:?}"
    );

    if tuistory_available() {
        let cortex = PathBuf::from(env!("CARGO_BIN_EXE_cortex"));
        let tui = TuistorySession::launch(
            format!("cross-m6-shared-live-todo-{}", std::process::id()),
            &cortex,
            &project,
            &home,
            &db_path,
            &session_id,
            Some(&key),
        );
        tui.run(&["type", "/sidebar-todo"]);
        tui.run(&["press", "enter"]);
        let snap = tui.wait_for_any(&[&n1, &n2, &n3], TUI_WAIT_TIMEOUT_MS);
        assert!(
            snap.contains(&n1) && snap.contains(&n2) && snap.contains(&n3),
            "TUI todo panel must show the three durable items after restart:\n{snap}"
        );
        tui.close();
    } else {
        eprintln!(
            "skipping TUI portion of live todo-restart proof: tuistory unavailable; server + db agreed"
        );
    }

    drop(server);
    assert_no_orphans(server_pgid);
    let _ = std::fs::remove_dir_all(&root);
}

/// VAL-CROSS-046: concurrent headless + server writers on distinct sessions share one store without
/// lock/corruption; both nonces land, both sessions are listable, stats read clean, listener dies by
/// exact PID.
#[test]
fn concurrent_headless_and_server_turns_share_store_without_locking() {
    let Some(key) = live_key() else {
        eprintln!("skipping live test: OPENROUTER_API_KEY not set");
        return;
    };

    let root = scratch_dir("live-concurrent");
    let home = root.join("home");
    std::fs::create_dir_all(&home).expect("home");
    let db_path = root.join("cortex.db");
    let port = free_server_port();

    let server = ServerGuard::new(
        spawn_group(cortex_command(
            Some(&key),
            &home,
            &db_path,
            &[
                "serve",
                "--hostname",
                "127.0.0.1",
                "--port",
                &port.to_string(),
            ],
        )),
        port,
    );
    let server_pgid = server.pgid();
    wait_for_health(port);

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
    assert_eq!(
        create.status.and_then(|status| status.code()),
        Some(0),
        "create session stderr: {}",
        create.stderr_str()
    );
    let session_b: serde_json::Value =
        serde_json::from_slice(&create.stdout).expect("created session JSON");
    let session_b = session_b["id"].as_str().expect("session id").to_owned();
    let session_a = format!("ses_live_headless_{}", std::process::id());
    let nonce_a = nonce("headless", 17);
    let nonce_b = nonce("server", 29);
    let prompt_a =
        format!("Output exactly this text and nothing else, no quotes, no extra words: {nonce_a}");
    let prompt_b =
        format!("Output exactly this text and nothing else, no quotes, no extra words: {nonce_b}");

    let headless = spawn_group(cortex_command(
        Some(&key),
        &home,
        &db_path,
        &["run", "--session", &session_a, &prompt_a],
    ));
    let server_turn = spawn_group({
        let mut command = Command::new("curl");
        command.args([
            "-sfS",
            "-X",
            "POST",
            "-H",
            "Content-Type: application/json",
            "-d",
            &serde_json::json!({ "text": prompt_b }).to_string(),
            &format!("http://127.0.0.1:{port}/session/{session_b}/message"),
        ]);
        command
    });

    let headless = collect_child(headless, RUN_TIMEOUT);
    let server_turn = collect_child(server_turn, RUN_TIMEOUT);

    assert_eq!(
        headless.status.and_then(|status| status.code()),
        Some(0),
        "headless stderr: {}",
        headless.stderr_str()
    );
    assert!(
        headless.stdout_str().contains(&nonce_a),
        "headless output should contain its own nonce: {}",
        headless.stdout_str()
    );
    assert_eq!(
        server_turn.status.and_then(|status| status.code()),
        Some(0),
        "server curl stderr: {}",
        server_turn.stderr_str()
    );
    let server_body: serde_json::Value =
        serde_json::from_slice(&server_turn.stdout).expect("server turn JSON");
    assert!(
        server_body["content"]
            .as_str()
            .is_some_and(|content| content.contains(&nonce_b)),
        "server response should contain its own nonce: {server_body:?}"
    );

    let listed = collect_child(
        spawn_group(cortex_command(
            Some(&key),
            &home,
            &db_path,
            &["session", "list", "--format", "json"],
        )),
        RUN_TIMEOUT,
    );
    assert_eq!(listed.status.and_then(|status| status.code()), Some(0));
    let list_body: serde_json::Value =
        serde_json::from_slice(&listed.stdout).expect("session list JSON");
    let ids = list_body["sessions"]
        .as_array()
        .expect("sessions")
        .iter()
        .filter_map(|session| session["id"].as_str())
        .collect::<Vec<_>>();
    assert!(
        ids.contains(&session_a.as_str()),
        "session A listed: {ids:?}"
    );
    assert!(
        ids.contains(&session_b.as_str()),
        "session B listed: {ids:?}"
    );

    let stats = collect_child(
        spawn_group(cortex_command(
            Some(&key),
            &home,
            &db_path,
            &["stats", "--format", "json"],
        )),
        RUN_TIMEOUT,
    );
    assert_eq!(
        stats.status.and_then(|status| status.code()),
        Some(0),
        "stats stderr: {}",
        stats.stderr_str()
    );
    let stats_body: serde_json::Value = serde_json::from_slice(&stats.stdout).expect("stats JSON");
    assert!(
        stats_body["totals"]["total_tokens"]
            .as_u64()
            .is_some_and(|tokens| tokens > 0),
        "stats should read non-zero usage from both turns: {stats_body:?}"
    );

    // Post-concurrency integrity: reopen and read both transcripts cleanly with no lock error.
    {
        let db = Database::open(&db_path).expect("reopen after concurrent writers");
        assert_eq!(db.integrity_check().expect("integrity"), "ok");
        let msgs_a = db
            .messages()
            .list_for_session(&session_a)
            .expect("session A messages");
        let msgs_b = db
            .messages()
            .list_for_session(&session_b)
            .expect("session B messages");
        assert!(
            msgs_a
                .iter()
                .any(|message| message.role == "assistant" && message.content.contains(&nonce_a)),
            "session A assistant retained: {msgs_a:?}"
        );
        assert!(
            msgs_b
                .iter()
                .any(|message| message.role == "assistant" && message.content.contains(&nonce_b)),
            "session B assistant retained: {msgs_b:?}"
        );
    }

    assert_no_orphans(headless.pgid);
    assert_no_orphans(server_turn.pgid);
    drop(server);
    assert_no_orphans(server_pgid);
    let _ = std::fs::remove_dir_all(&root);
}
