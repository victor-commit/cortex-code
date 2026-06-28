//! VAL-GAP-DATA-010: the environment service supports get/all/set/remove with redaction.
//!
//! Part A drives the `cortex env` CLI on an isolated, per-workspace scoped store and asserts
//! set/get/remove round-trip and that `all`/`list` redacts secret-looking values. Part B proves the
//! "removed keys no longer affect runs" half end-to-end: a value set with `cortex env set` reaches a
//! command run through the real `cortex serve` shell route, and after `cortex env remove` the same
//! command no longer sees it. The loopback server uses only the mission port range 3200-3299 and is
//! stopped by its exact listener PID.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Child, Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use serde_json::Value;

static SCRATCH_COUNTER: AtomicU64 = AtomicU64::new(0);
const WAIT_TIMEOUT: Duration = Duration::from_secs(10);

struct Workspace {
    root: PathBuf,
}

impl Workspace {
    fn new(label: &str) -> Self {
        let id = SCRATCH_COUNTER.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "cortex-env-svc-{label}-{}-{id}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("home")).expect("home");
        std::fs::create_dir_all(root.join("work")).expect("work");
        std::fs::create_dir_all(root.join("data")).expect("data");
        Self { root }
    }

    fn home(&self) -> PathBuf {
        self.root.join("home")
    }

    fn work(&self) -> PathBuf {
        self.root.join("work")
    }

    fn data(&self) -> PathBuf {
        self.root.join("data")
    }

    fn db_path(&self) -> PathBuf {
        self.root.join("cortex.db")
    }

    /// A `cortex` command rooted in the shared workspace directory and isolated data dir, so the
    /// `env` CLI and the `serve` process resolve the same per-workspace scoped store.
    fn command(&self, args: &[&str]) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_cortex"));
        command.env_clear();
        command.current_dir(self.work());
        command.env("HOME", self.home());
        command.env("CORTEX_DATA_DIR", self.data());
        command.env("CORTEX_DB_PATH", self.db_path());
        command.env("PATH", std::env::var("PATH").unwrap_or_default());
        command.args(args);
        command
    }

    fn run(&self, args: &[&str]) -> Output {
        let mut command = self.command(args);
        command.stdin(Stdio::null());
        command.stdout(Stdio::piped());
        command.stderr(Stdio::piped());
        command
            .spawn()
            .expect("spawn cortex")
            .wait_with_output()
            .expect("collect output")
    }

    fn run_ok(&self, args: &[&str]) -> Value {
        let output = self.run(args);
        assert!(
            output.status.success(),
            "command {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).unwrap_or_else(|err| {
            panic!(
                "stdout for {args:?} was not JSON ({err}): {}",
                String::from_utf8_lossy(&output.stdout)
            )
        })
    }
}

impl Drop for Workspace {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[test]
fn env_cli_set_get_remove_round_trips_and_redacts_secret_values() {
    let workspace = Workspace::new("cli");

    let set = workspace.run_ok(&[
        "env",
        "set",
        "PROJECT_REGION",
        "us-west-2",
        "--format",
        "json",
    ]);
    assert_eq!(set["status"], "set");
    assert_eq!(set["secret"], false);

    // A clearly API-key-shaped value, assembled at runtime so the secret scanner never sees a
    // key-shaped literal in source.
    let secret = format!("sk-{}-{}", "or-v1", "a".repeat(48));
    let set_secret = workspace.run_ok(&[
        "env",
        "set",
        "OPENROUTER_API_KEY",
        &secret,
        "--format",
        "json",
    ]);
    assert_eq!(
        set_secret["secret"], true,
        "an api-key-shaped value is flagged secret"
    );

    let got = workspace.run(&["env", "get", "PROJECT_REGION"]);
    assert!(got.status.success());
    assert_eq!(
        String::from_utf8_lossy(&got.stdout).trim(),
        "us-west-2",
        "get prints the literal value verbatim"
    );

    let listed = workspace.run_ok(&["env", "all", "--format", "json"]);
    let values = listed["values"].as_array().expect("values array");
    let region = values
        .iter()
        .find(|entry| entry["key"] == "PROJECT_REGION")
        .expect("region entry");
    assert_eq!(
        region["value"], "us-west-2",
        "plain value shown verbatim in list"
    );
    let key_entry = values
        .iter()
        .find(|entry| entry["key"] == "OPENROUTER_API_KEY")
        .expect("secret entry");
    assert_eq!(key_entry["secret"], true);
    assert_ne!(
        key_entry["value"], secret,
        "a secret-looking value must be redacted in the list output"
    );

    // The list text output must never leak the raw secret either.
    let listed_text = workspace.run(&["env", "all"]);
    let text = String::from_utf8_lossy(&listed_text.stdout);
    assert!(
        !text.contains(&secret),
        "list text must not contain the raw secret: {text}"
    );

    let removed = workspace.run_ok(&["env", "remove", "PROJECT_REGION", "--format", "json"]);
    assert_eq!(removed["status"], "removed");

    let missing = workspace.run(&["env", "get", "PROJECT_REGION"]);
    assert!(
        !missing.status.success(),
        "a removed key is no longer gettable"
    );
}

#[test]
fn removed_env_keys_no_longer_affect_server_shell_runs() {
    let workspace = Workspace::new("affect-runs");
    let key = "CORTEX_ENV_WIRED_MARKER";
    let value = "envwiredmarker7a2c9";

    // Set the scoped value, then start the server in the same workspace + data dir.
    let set = workspace.run_ok(&["env", "set", key, value, "--format", "json"]);
    assert_eq!(set["status"], "set");

    let port = free_port();
    let mut server = ServerProcess::start(&workspace, port);

    let session_id = create_session(port);

    let before = shell_output(port, &session_id, &format!("printenv {key} || true"));
    assert!(
        before.contains(value),
        "a value set via `cortex env set` must reach the server shell run, got: {before:?}"
    );

    // Remove the key; the same command must no longer see it.
    let removed = workspace.run_ok(&["env", "remove", key, "--format", "json"]);
    assert_eq!(removed["status"], "removed");

    let after = shell_output(port, &session_id, &format!("printenv {key} || true"));
    assert!(
        !after.contains(value),
        "a removed key must no longer affect later runs, but still saw it: {after:?}"
    );

    server.stop();
    wait_for_port_released(port);
}

struct ServerProcess {
    child: Child,
    port: u16,
    stopped: bool,
}

impl ServerProcess {
    fn start(workspace: &Workspace, port: u16) -> Self {
        let port_arg = port.to_string();
        let mut command = workspace.command(&[
            "serve",
            "--hostname",
            "127.0.0.1",
            "--port",
            port_arg.as_str(),
        ]);
        command.stdin(Stdio::null());
        command.stdout(Stdio::piped());
        command.stderr(Stdio::piped());
        let mut child = command.spawn().expect("spawn cortex serve");
        wait_for_health(port, &mut child);
        Self {
            child,
            port,
            stopped: false,
        }
    }

    fn stop(&mut self) {
        if self.stopped {
            return;
        }
        if let Some(pid) = listener_pid(self.port) {
            let _ = Command::new("kill").arg(pid.to_string()).status();
        }
        let deadline = Instant::now() + WAIT_TIMEOUT;
        while Instant::now() < deadline {
            if let Ok(Some(_)) = self.child.try_wait() {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
        self.stopped = true;
    }
}

impl Drop for ServerProcess {
    fn drop(&mut self) {
        self.stop();
    }
}

fn create_session(port: u16) -> String {
    let response = request(port, "POST", "/session", Some("{}"));
    assert_eq!(response.status, 201, "create session: {}", response.body);
    let value: Value = serde_json::from_str(&response.body).expect("session JSON");
    value["id"].as_str().expect("session id").to_owned()
}

fn shell_output(port: u16, session_id: &str, command: &str) -> String {
    let body = serde_json::json!({ "command": command }).to_string();
    let response = request(
        port,
        "POST",
        &format!("/session/{session_id}/shell"),
        Some(&body),
    );
    assert_eq!(response.status, 200, "shell run failed: {}", response.body);
    let value: Value = serde_json::from_str(&response.body).expect("shell JSON");
    value["output"].as_str().unwrap_or_default().to_owned()
}

struct HttpResponse {
    status: u16,
    body: String,
}

fn request(port: u16, method: &str, path: &str, body: Option<&str>) -> HttpResponse {
    let addr: SocketAddr = format!("127.0.0.1:{port}").parse().expect("addr");
    let mut stream =
        TcpStream::connect_timeout(&addr, Duration::from_millis(500)).expect("connect");
    stream
        .set_read_timeout(Some(Duration::from_secs(15)))
        .expect("read timeout");
    stream
        .set_write_timeout(Some(Duration::from_secs(5)))
        .expect("write timeout");
    let body = body.unwrap_or("");
    let mut request =
        format!("{method} {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n");
    if !body.is_empty() {
        request.push_str("Content-Type: application/json\r\n");
        request.push_str(&format!("Content-Length: {}\r\n", body.len()));
    }
    request.push_str("\r\n");
    request.push_str(body);
    stream.write_all(request.as_bytes()).expect("write request");

    let mut raw = String::new();
    stream.read_to_string(&mut raw).expect("read response");
    let (head, body) = raw.split_once("\r\n\r\n").unwrap_or((raw.as_str(), ""));
    let status = head
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|code| code.parse().ok())
        .unwrap_or(0);
    HttpResponse {
        status,
        body: body.to_owned(),
    }
}

fn wait_for_health(port: u16, child: &mut Child) {
    let deadline = Instant::now() + WAIT_TIMEOUT;
    while Instant::now() < deadline {
        if let Ok(Some(status)) = child.try_wait() {
            panic!("serve exited before health on port {port}: {status}");
        }
        if let Ok(mut stream) = TcpStream::connect_timeout(
            &format!("127.0.0.1:{port}").parse().unwrap(),
            Duration::from_millis(200),
        ) {
            let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
            let _ = stream
                .write_all(b"GET /health HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n");
            let mut raw = String::new();
            if stream.read_to_string(&mut raw).is_ok() && raw.contains("200") {
                return;
            }
        }
        std::thread::sleep(Duration::from_millis(75));
    }
    panic!("serve on port {port} never became healthy");
}

fn free_port() -> u16 {
    for port in (3200u16..=3299).rev() {
        if listener_pid(port).is_none() && TcpListener::bind(("127.0.0.1", port)).is_ok() {
            return port;
        }
    }
    panic!("no free loopback port in 3200-3299");
}

fn listener_pid(port: u16) -> Option<u32> {
    let output = Command::new("lsof")
        .arg(format!("-tiTCP:{port}"))
        .arg("-sTCP:LISTEN")
        .output()
        .ok()?;
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .find_map(|line| line.trim().parse().ok())
}

fn wait_for_port_released(port: u16) {
    let deadline = Instant::now() + WAIT_TIMEOUT;
    while Instant::now() < deadline {
        if listener_pid(port).is_none() && TcpListener::bind(("127.0.0.1", port)).is_ok() {
            return;
        }
        std::thread::sleep(Duration::from_millis(75));
    }
    panic!("port {port} still has a listener after shutdown");
}
