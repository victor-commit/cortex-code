//! Cross-surface session visibility checks for the M6 server/ACP integration slice.
//!
//! These black-box tests keep one scratch SQLite store and cross the real surface seams:
//! server routes, `cortex session`, and `cortex acp` stdio. They are deliberately offline and
//! deterministic, while the worker handoff covers the live `cortex run` + curl proof.

use std::fs;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::{Child, Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode, header};
use db::Session;
use serde_json::{Value, json};
use tower::ServiceExt;

const RUN_TIMEOUT: Duration = Duration::from_secs(20);

static SCRATCH_COUNTER: AtomicU64 = AtomicU64::new(0);

struct Scratch {
    root: PathBuf,
}

impl Scratch {
    fn new(label: &str) -> Self {
        let id = SCRATCH_COUNTER.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "cortex-cross-m6-session-{label}-{}-{id}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("home")).expect("create scratch tree");
        Self { root }
    }

    fn home(&self) -> PathBuf {
        self.root.join("home")
    }

    fn db_path(&self) -> PathBuf {
        self.root.join("cortex.db")
    }

    fn command(&self, args: &[&str]) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_cortex"));
        command.env_clear();
        command.env("HOME", self.home());
        command.env("CORTEX_DB_PATH", self.db_path());
        command.args(args);
        command
    }

    fn run(&self, args: &[&str]) -> Output {
        collect(spawn(self.command(args), None))
    }

    fn run_with_stdin(&self, args: &[&str], stdin: impl AsRef<[u8]>) -> Output {
        collect(spawn(self.command(args), Some(stdin.as_ref().to_vec())))
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn spawn(mut command: Command, stdin: Option<Vec<u8>>) -> Child {
    command.stdin(Stdio::piped());
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());
    let mut child = command.spawn().expect("spawn cortex binary");
    if let Some(bytes) = stdin {
        let mut input = child.stdin.take().expect("stdin handle");
        std::thread::spawn(move || {
            input.write_all(&bytes).expect("write child stdin");
        });
    } else {
        drop(child.stdin.take());
    }
    child
}

fn collect(mut child: Child) -> Output {
    let mut out = child.stdout.take().expect("stdout handle");
    let mut err = child.stderr.take().expect("stderr handle");
    let out_thread = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = out.read_to_end(&mut buf);
        buf
    });
    let err_thread = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = err.read_to_end(&mut buf);
        buf
    });

    let start = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().expect("try_wait") {
            break status;
        }
        if start.elapsed() > RUN_TIMEOUT {
            let _ = child.kill();
            let _ = child.wait();
            panic!("cortex process did not terminate within {RUN_TIMEOUT:?}");
        }
        std::thread::sleep(Duration::from_millis(20));
    };

    Output {
        status,
        stdout: out_thread.join().expect("join stdout"),
        stderr: err_thread.join().expect("join stderr"),
    }
}

fn stderr_str(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn parse_stdout_json(output: &Output) -> Value {
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        stderr_str(output)
    );
    serde_json::from_slice(&output.stdout).expect("stdout JSON")
}

fn parse_stdout_json_lines(output: &Output) -> Vec<Value> {
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        stderr_str(output)
    );
    String::from_utf8(output.stdout.clone())
        .expect("utf8 stdout")
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str(line).expect("json line"))
        .collect()
}

fn session_ids_from_cli_list(scratch: &Scratch) -> Vec<String> {
    let listed = scratch.run(&["session", "list", "--format", "json"]);
    let body = parse_stdout_json(&listed);
    body["sessions"]
        .as_array()
        .expect("sessions array")
        .iter()
        .map(|session| session["id"].as_str().expect("session id").to_owned())
        .collect()
}

async fn server_create_session(db_path: PathBuf, title: &str) -> Session {
    let app = server::app_with_db_path(db_path);
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/session")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(json!({ "title": title }).to_string()))
                .expect("request"),
        )
        .await
        .expect("create response");
    assert_eq!(response.status(), StatusCode::CREATED);
    json_body(response).await
}

async fn json_body<T>(response: axum::response::Response) -> T
where
    T: serde::de::DeserializeOwned,
{
    let bytes = to_bytes(response.into_body(), 16_384).await.expect("body");
    serde_json::from_slice(&bytes).expect("JSON body")
}

// VAL-CROSS-003 (headless-list half): a session created through the server API lands in the shared
// store that `cortex session list` reads, rather than staying private to the HTTP surface.
#[tokio::test]
async fn server_created_session_is_visible_to_headless_session_list() {
    let scratch = Scratch::new("server-to-headless");
    let created = server_create_session(scratch.db_path(), "server api marker").await;

    let ids = session_ids_from_cli_list(&scratch);

    assert!(
        ids.contains(&created.id),
        "server-created session {} should be listed by `cortex session`: {ids:?}",
        created.id
    );
}

// VAL-CROSS-005 (offline seam): messages written by the headless session CLI are retrievable
// through the server message route on the same store.
#[tokio::test]
async fn headless_created_session_messages_are_fetchable_through_server() {
    let scratch = Scratch::new("headless-to-server");
    let created = scratch.run(&[
        "session",
        "new",
        "--title",
        "headless marker",
        "--format",
        "json",
    ]);
    let created = parse_stdout_json(&created);
    let session_id = created["id"].as_str().expect("session id").to_owned();

    for (role, content) in [
        ("user", "headless user marker"),
        ("assistant", "headless assistant marker"),
    ] {
        let stored = scratch.run(&[
            "session",
            "message",
            &session_id,
            content,
            "--role",
            role,
            "--format",
            "json",
        ]);
        parse_stdout_json(&stored);
    }

    let app = server::app_with_db_path(scratch.db_path());
    let response = app
        .oneshot(
            Request::builder()
                .uri(format!("/session/{session_id}/message"))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("message list response");
    assert_eq!(response.status(), StatusCode::OK);
    let body: Value = json_body(response).await;

    assert_eq!(body["session"]["id"], session_id);
    let messages = body["messages"].as_array().expect("messages array");
    assert!(
        messages.iter().any(
            |message| message["role"] == "user" && message["content"] == "headless user marker"
        ),
        "server should return the headless user message: {body:?}"
    );
    assert!(
        messages.iter().any(|message| message["role"] == "assistant"
            && message["content"] == "headless assistant marker"),
        "server should return the headless assistant message: {body:?}"
    );
}

// VAL-CROSS-044 (headless-list half): `cortex acp` session/new persists to the same store that the
// ordinary headless session list enumerates.
#[test]
fn acp_created_session_is_visible_to_headless_session_list() {
    let scratch = Scratch::new("acp-to-headless");
    let input = format!(
        "{}\n{}\n",
        json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": 1,
                "clientCapabilities": {},
                "clientInfo": { "name": "cross-m6-test", "version": "0" }
            }
        }),
        json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "session/new",
            "params": {
                "cwd": scratch.root,
                "mcpServers": []
            }
        })
    );

    let acp = scratch.run_with_stdin(&["acp"], input);
    let values = parse_stdout_json_lines(&acp);
    let session_id = values
        .iter()
        .find(|value| value.get("id") == Some(&json!(2)))
        .and_then(|value| value.pointer("/result/sessionId"))
        .and_then(Value::as_str)
        .expect("ACP session id")
        .to_owned();

    let ids = session_ids_from_cli_list(&scratch);

    assert!(
        ids.contains(&session_id),
        "ACP-created session {session_id} should be listed by `cortex session`: {ids:?}"
    );
}
