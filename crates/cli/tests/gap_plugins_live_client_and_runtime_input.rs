use std::fs;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::Value;

const WAIT_TIMEOUT: Duration = Duration::from_secs(10);
const RUN_TIMEOUT: Duration = Duration::from_secs(30);

#[test]
fn plugin_client_context_and_runtime_state_are_live() {
    let scratch = Scratch::new("gap-plugin-live-client");
    let port = free_port();
    let server_url = format!("http://127.0.0.1:{port}");

    let mut serve = spawn_serve(&scratch, port);
    wait_for_health(port, &mut serve);
    let listener_pid = listener_pid(port).expect("serve listener pid");
    assert_eq!(
        listener_pid,
        serve.id(),
        "serve pid should own the listener"
    );

    let worktree_root = init_worktree_repo(&scratch);
    let nested = worktree_root.join("nested").join("child");
    fs::create_dir_all(&nested).expect("create nested worktree dir");

    let log_path = worktree_root.join("plugin-log.jsonl");
    write_plugin(&worktree_root, &log_path);

    let run = run_cortex(
        &scratch,
        &nested,
        &[
            "run",
            "--command",
            "echo",
            "--command-arg",
            "plugin-client-live",
        ],
        &[("CORTEX_SERVER_URL", server_url.as_str())],
    );
    assert_success("cortex run", &run);

    let log = fs::read_to_string(&log_path).expect("plugin log");
    let entries: Vec<Value> = log
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str(line).expect("valid plugin jsonl line"))
        .collect();
    assert!(
        entries.len() >= 3,
        "expected event/command/dispose hooks, got: {entries:#?}"
    );
    let event = entries
        .iter()
        .find(|entry| entry["hook"] == "event")
        .expect("event hook record");
    let command = entries
        .iter()
        .find(|entry| entry["hook"] == "command.execute.before")
        .expect("command hook record");
    let dispose = entries
        .iter()
        .find(|entry| entry["hook"] == "dispose")
        .expect("dispose hook record");

    assert_eq!(event["counter"], 1, "event should be first hook");
    assert_eq!(
        command["counter"], 2,
        "command hook should reuse module state"
    );
    assert_eq!(
        dispose["counter"], 3,
        "dispose hook should reuse module state"
    );

    let created_id = event["createdId"].as_str().expect("created session id");
    assert!(
        !created_id.is_empty(),
        "event hook should create a session through the live client"
    );
    assert!(
        event["listedCount"]
            .as_u64()
            .is_some_and(|count| count >= 1),
        "session list through the live client should be non-empty: {event:#?}"
    );

    let sessions = run_cortex(
        &scratch,
        &nested,
        &["session", "list", "--format", "json"],
        &[],
    );
    assert_success("cortex session list", &sessions);
    let sessions_json: Value = serde_json::from_slice(&sessions.stdout).expect("session list json");
    let listed = sessions_json["sessions"]
        .as_array()
        .expect("session list array");
    assert!(
        listed
            .iter()
            .any(|session| session["id"].as_str() == Some(created_id)),
        "client-created session id should be visible to the shipped CLI session API"
    );

    let nested_canon = fs::canonicalize(&nested).expect("canonical nested");
    let worktree_canon = fs::canonicalize(&worktree_root).expect("canonical worktree");
    assert_eq!(
        PathBuf::from(event["directory"].as_str().unwrap_or_default()),
        nested_canon
    );
    assert_eq!(
        PathBuf::from(event["root"].as_str().unwrap_or_default()),
        worktree_canon
    );
    assert_eq!(
        PathBuf::from(event["projectRoot"].as_str().unwrap_or_default()),
        worktree_canon
    );
    assert_eq!(event["serverUrl"].as_str(), Some(server_url.as_str()));
    assert!(
        event["worktree"].as_str().is_some_and(
            |worktree| !worktree.is_empty() && worktree != nested_canon.display().to_string()
        ),
        "worktree identity should be real and distinct from cwd: {event:#?}"
    );

    kill_pid(listener_pid);
    wait_for_child_exit(&mut serve, WAIT_TIMEOUT);
    wait_for_port_released(port);
}

fn write_plugin(root: &Path, log_path: &Path) {
    let plugin_path = root.join("plugin.mjs");
    fs::write(
        &plugin_path,
        r#"
import { appendFile } from "node:fs/promises";

let counter = 0;

export default async function plugin(input, options) {
  async function record(hook, extra = {}) {
    counter += 1;
    await appendFile(options.log, JSON.stringify({
      hook,
      counter,
      directory: input.directory,
      root: input.root,
      projectRoot: input.project?.root,
      worktree: input.worktree,
      serverUrl: input.serverUrl,
      ...extra
    }) + "\n");
  }

  return {
    event: async (event) => {
      const created = await input.client.session.create({ title: "plugin-live-session" });
      const listed = await input.client.session.list();
      await record("event", {
        eventKind: event.kind,
        createdId: created?.id ?? null,
        listedCount: Array.isArray(listed) ? listed.length : null
      });
    },
    "command.execute.before": async (command) => {
      await record("command.execute.before", { command: command.name ?? command.command ?? null });
      return command;
    },
    dispose: async () => {
      await record("dispose");
    }
  };
}
"#,
    )
    .expect("write plugin");
    fs::write(
        root.join("cortex.toml"),
        format!(
            "plugin = [[\"{}\", {{ log = \"{}\" }}]]\n",
            plugin_path.display(),
            log_path.display()
        ),
    )
    .expect("write plugin config");
}

fn init_worktree_repo(scratch: &Scratch) -> PathBuf {
    let repo = scratch.root.join("repo-main");
    fs::create_dir_all(&repo).expect("create main repo");
    run_command(&repo, "git", &["init"]);
    run_command(
        &repo,
        "git",
        &["config", "user.email", "cortex@example.com"],
    );
    run_command(&repo, "git", &["config", "user.name", "Cortex"]);
    fs::write(repo.join("seed.txt"), "seed\n").expect("write seed");
    run_command(&repo, "git", &["add", "seed.txt"]);
    run_command(&repo, "git", &["commit", "-m", "initial"]);

    let worktree = scratch.root.join("repo-worktree");
    let worktree_arg = worktree.to_string_lossy().to_string();
    run_command(&repo, "git", &["worktree", "add", &worktree_arg]);
    worktree
}

fn run_command(cwd: &Path, program: &str, args: &[&str]) {
    let output = Command::new(program)
        .args(args)
        .current_dir(cwd)
        .output()
        .expect("run command");
    let rendered = if args.is_empty() {
        program.to_owned()
    } else {
        format!("{program} {}", args.join(" "))
    };
    assert!(
        output.status.success(),
        "command failed: {rendered}\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn run_cortex(scratch: &Scratch, cwd: &Path, args: &[&str], extra_env: &[(&str, &str)]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_cortex"));
    command.current_dir(cwd);
    command.args(args);
    command.env("HOME", &scratch.home);
    command.env("CORTEX_DB_PATH", &scratch.db);
    command
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("XDG_DATA_HOME")
        .env_remove("CORTEX_DATA_DIR")
        .env_remove("CORTEX_NO_NET")
        .env_remove("OPENROUTER_API_KEY")
        .env_remove("CORTEX_OPENROUTER_BASE_URL")
        .env_remove("RUST_LOG")
        .env_remove("CORTEX_VERBOSE");
    for (name, value) in extra_env {
        command.env(name, value);
    }
    run_to_output(command, RUN_TIMEOUT)
}

fn spawn_serve(scratch: &Scratch, port: u16) -> Child {
    let mut command = Command::new(env!("CARGO_BIN_EXE_cortex"));
    command.current_dir(&scratch.root);
    command.args([
        "serve",
        "--hostname",
        "127.0.0.1",
        "--port",
        &port.to_string(),
    ]);
    command.env("HOME", &scratch.home);
    command.env("CORTEX_DB_PATH", &scratch.db);
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("XDG_DATA_HOME")
        .env_remove("CORTEX_DATA_DIR")
        .env_remove("CORTEX_NO_NET");
    command.spawn().expect("spawn serve")
}

fn wait_for_health(port: u16, child: &mut Child) {
    let started = Instant::now();
    let mut last_error = String::new();
    while started.elapsed() < WAIT_TIMEOUT {
        if let Some(status) = child.try_wait().expect("serve try_wait") {
            panic!("serve exited before health check: {status}");
        }
        match request_http(port, "GET", "/health", None) {
            Ok(response) if response.status == 200 => return,
            Ok(response) => last_error = format!("status {}", response.status),
            Err(err) => last_error = err.to_string(),
        }
        thread::sleep(Duration::from_millis(50));
    }
    panic!("serve never became healthy: {last_error}");
}

struct HttpResponse {
    status: u16,
}

fn request_http(
    port: u16,
    method: &str,
    path: &str,
    body: Option<&str>,
) -> std::io::Result<HttpResponse> {
    let addr: SocketAddr = format!("127.0.0.1:{port}").parse().expect("valid addr");
    let mut stream = TcpStream::connect_timeout(&addr, Duration::from_millis(250))?;
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    stream.set_write_timeout(Some(Duration::from_secs(2)))?;
    let body = body.unwrap_or("");
    let mut request =
        format!("{method} {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n");
    if !body.is_empty() {
        request.push_str("Content-Type: application/json\r\n");
        request.push_str(&format!("Content-Length: {}\r\n", body.len()));
    }
    request.push_str("\r\n");
    request.push_str(body);
    stream.write_all(request.as_bytes())?;

    let mut raw = String::new();
    stream.read_to_string(&mut raw)?;
    let head = raw
        .split_once("\r\n\r\n")
        .map(|(h, _)| h)
        .unwrap_or(raw.as_str());
    let status = head
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|code| code.parse().ok())
        .unwrap_or(0);
    Ok(HttpResponse { status })
}

fn run_to_output(mut command: Command, timeout: Duration) -> Output {
    command.stdin(Stdio::null());
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());

    let mut child = command.spawn().expect("spawn command");
    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().expect("try_wait") {
            break status;
        }
        if started.elapsed() > timeout {
            let _ = child.kill();
            let _ = child.wait();
            panic!("command timed out after {timeout:?}");
        }
        thread::sleep(Duration::from_millis(20));
    };

    let mut stdout = Vec::new();
    if let Some(mut handle) = child.stdout.take() {
        handle.read_to_end(&mut stdout).expect("read stdout");
    }
    let mut stderr = Vec::new();
    if let Some(mut handle) = child.stderr.take() {
        handle.read_to_end(&mut stderr).expect("read stderr");
    }
    Output {
        status,
        stdout,
        stderr,
    }
}

fn assert_success(label: &str, output: &Output) {
    assert!(
        output.status.success(),
        "{label} failed with status {:?}\nstdout:\n{}\nstderr:\n{}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn free_port() -> u16 {
    for port in server::MIN_PORT..=server::MAX_PORT {
        if listener_pid(port).is_none() && TcpListener::bind(("127.0.0.1", port)).is_ok() {
            return port;
        }
    }
    panic!("no free port in 3200-3299");
}

fn listener_pid(port: u16) -> Option<u32> {
    let output = Command::new("lsof")
        .arg(format!("-tiTCP:{port}"))
        .arg("-sTCP:LISTEN")
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .find_map(|line| line.trim().parse().ok())
}

fn kill_pid(pid: u32) {
    let status = Command::new("kill")
        .arg(pid.to_string())
        .status()
        .expect("run kill");
    assert!(status.success(), "kill {pid} failed");
}

fn wait_for_child_exit(child: &mut Child, timeout: Duration) {
    let started = Instant::now();
    while started.elapsed() < timeout {
        if child.try_wait().expect("try_wait").is_some() {
            return;
        }
        thread::sleep(Duration::from_millis(50));
    }
    let _ = child.kill();
    let _ = child.wait();
    panic!("child did not exit within {timeout:?}");
}

fn wait_for_port_released(port: u16) {
    let started = Instant::now();
    while started.elapsed() < WAIT_TIMEOUT {
        if listener_pid(port).is_none() {
            assert!(
                TcpListener::bind(("127.0.0.1", port)).is_ok(),
                "port {port} has no listener but cannot be rebound"
            );
            return;
        }
        thread::sleep(Duration::from_millis(50));
    }
    panic!("port {port} still has a listener after shutdown");
}

struct Scratch {
    root: PathBuf,
    home: PathBuf,
    db: PathBuf,
}

impl Scratch {
    fn new(tag: &str) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let root =
            std::env::temp_dir().join(format!("cortex-{tag}-{}-{nanos}", std::process::id()));
        let home = root.join("home");
        fs::create_dir_all(&home).expect("create home");
        let db = root.join("cortex.db");
        Self { root, home, db }
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
