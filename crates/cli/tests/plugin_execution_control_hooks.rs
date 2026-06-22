//! Black-box plugin execution-control hook coverage through the shipped `cortex` binary.

use std::io::Read;
use std::net::{SocketAddr, TcpListener};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::sync::atomic::{AtomicU16, Ordering};
use std::thread;
use std::time::{Duration, Instant};

const RUN_TIMEOUT: Duration = Duration::from_secs(60);
const WAIT_TIMEOUT: Duration = Duration::from_secs(10);

static NEXT_PORT: AtomicU16 = AtomicU16::new(3270);

struct Scratch {
    root: PathBuf,
    home: PathBuf,
    project: PathBuf,
    db: PathBuf,
    audit: PathBuf,
}

impl Scratch {
    fn new(tag: &str) -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "cortex-plugin-exec-control-{tag}-{}-{nanos}",
            std::process::id()
        ));
        let home = root.join("home");
        let project = root.join("project");
        std::fs::create_dir_all(&home).expect("create scratch home");
        std::fs::create_dir_all(&project).expect("create scratch project");
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

struct ToolCallProvider {
    port: u16,
    child: Option<Child>,
    listener_pid: u32,
}

impl ToolCallProvider {
    fn spawn() -> Self {
        let port = free_port();
        let dir = std::env::temp_dir().join(format!(
            "cortex-plugin-toolcall-server-{}-{port}",
            std::process::id()
        ));
        let _ = std::fs::create_dir_all(&dir);
        let script = dir.join("server.py");
        std::fs::write(&script, TOOL_CALL_SERVER).expect("write tool-call server");
        let child = Command::new("python3")
            .arg(&script)
            .arg(port.to_string())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn tool-call provider");
        let listener_pid = wait_for_listener_pid(port);
        assert_eq!(listener_pid, child.id());
        Self {
            port,
            child: Some(child),
            listener_pid,
        }
    }

    fn base_url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    fn shutdown_and_assert_released(mut self) {
        kill_listener(self.listener_pid);
        if let Some(mut child) = self.child.take() {
            let _ = child.wait();
        }
        wait_for_port_released(self.port);
    }
}

impl Drop for ToolCallProvider {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            kill_listener(self.listener_pid);
            let _ = child.wait();
        }
    }
}

#[test]
fn tool_execute_before_veto_blocks_write_and_records_audit_denial() {
    let provider = ToolCallProvider::spawn();
    let scratch = Scratch::new("tool-veto");
    write_tool_veto_plugin(&scratch.project);

    let out = run(cortex_in(
        &scratch,
        Some(&provider),
        &["run", "--format", "json", "please write target.txt"],
    ));
    assert_success(&out);

    assert!(
        !scratch.project.join("target.txt").exists(),
        "the plugin veto must prevent the write side effect"
    );
    let audit = std::fs::read_to_string(&scratch.audit).expect("audit denial record");
    assert!(audit.contains("\"type\":\"plugin.tool.denial\""), "{audit}");
    assert!(audit.contains("\"outcome\":\"denied\""), "{audit}");
    assert!(audit.contains("\"tool\":\"write\""), "{audit}");
    assert!(audit.contains("tool.execute.before"), "{audit}");
    for line in audit.lines().filter(|line| !line.trim().is_empty()) {
        let record: serde_json::Value = serde_json::from_str(line).expect("audit json");
        assert!(
            !(record["type"] == "file_mutation"
                && record["path"]
                    .as_str()
                    .is_some_and(|path| path.ends_with("target.txt"))),
            "a vetoed write must not produce a file mutation record: {audit}"
        );
    }
    provider.shutdown_and_assert_released();
}

#[test]
fn command_execute_before_can_veto_transform_and_dispose_runs_on_exit() {
    let scratch = Scratch::new("command");
    write_command_plugin(&scratch.project);

    let vetoed = run(cortex_in(
        &scratch,
        None,
        &[
            "run",
            "--command",
            "touch",
            "--command-arg",
            "blocked.marker",
        ],
    ));
    assert!(
        !vetoed.status.success(),
        "vetoed command exits non-zero\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&vetoed.stdout),
        String::from_utf8_lossy(&vetoed.stderr)
    );
    assert!(!scratch.project.join("blocked.marker").exists());

    let transformed = run(cortex_in(
        &scratch,
        None,
        &[
            "run",
            "--command",
            "touch",
            "--command-arg",
            "original.marker",
        ],
    ));
    assert_success(&transformed);
    assert!(!scratch.project.join("original.marker").exists());
    assert!(
        scratch.project.join("rewritten.marker").exists(),
        "command hook should rewrite the touch target"
    );
    assert!(
        scratch.project.join("disposed.marker").exists(),
        "dispose hook should run before the clean command process exits"
    );
}

fn write_tool_veto_plugin(project: &Path) {
    std::fs::write(
        project.join("plugin.mjs"),
        r#"
export default async function plugin() {
  return {
    "tool.execute.before": async (call) => {
      if (call.name === "write") {
        return { veto: true, reason: "blocked by plugin" };
      }
      return call;
    }
  };
}
"#,
    )
    .expect("write plugin");
    std::fs::write(project.join("cortex.toml"), "plugin = [\"./plugin.mjs\"]\n")
        .expect("write config");
}

fn write_command_plugin(project: &Path) {
    std::fs::write(
        project.join("plugin.mjs"),
        r#"
import { writeFile } from "node:fs/promises";

export default async function plugin(input) {
  return {
    "command.execute.before": async (command) => {
      if (command.args?.[0] === "blocked.marker") {
        return { veto: true, reason: "blocked command" };
      }
      if (command.args?.[0] === "original.marker") {
        return { ...command, args: ["rewritten.marker"] };
      }
      return command;
    },
    dispose: async () => {
      await writeFile(`${input.directory}/disposed.marker`, "disposed");
    }
  };
}
"#,
    )
    .expect("write plugin");
    std::fs::write(project.join("cortex.toml"), "plugin = [\"./plugin.mjs\"]\n")
        .expect("write config");
}

fn cortex_in(scratch: &Scratch, provider: Option<&ToolCallProvider>, args: &[&str]) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_cortex"));
    command.current_dir(&scratch.project);
    command.args(args);
    command.env("HOME", &scratch.home);
    command.env("CORTEX_DB_PATH", &scratch.db);
    command.env("CORTEX_AUDIT_LOG", &scratch.audit);
    if let Some(provider) = provider {
        command.env("OPENROUTER_API_KEY", fake_openrouter_key());
        command.env("CORTEX_OPENROUTER_BASE_URL", provider.base_url());
    } else {
        command.env_remove("OPENROUTER_API_KEY");
        command.env_remove("CORTEX_OPENROUTER_BASE_URL");
    }
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

fn run(mut command: Command) -> Output {
    command.stdin(Stdio::null());
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());

    let mut child = command.spawn().expect("spawn cortex binary");
    let mut out = child.stdout.take().expect("stdout handle");
    let mut err = child.stderr.take().expect("stderr handle");
    let out_thread = thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = out.read_to_end(&mut buf);
        buf
    });
    let err_thread = thread::spawn(move || {
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
            panic!("cortex run did not terminate within {RUN_TIMEOUT:?}");
        }
        thread::sleep(Duration::from_millis(20));
    };

    Output {
        status,
        stdout: out_thread.join().expect("join stdout reader"),
        stderr: err_thread.join().expect("join stderr reader"),
    }
}

fn assert_success(out: &Output) {
    assert!(
        out.status.success(),
        "command failed with status {:?}\nstdout:\n{}\nstderr:\n{}",
        out.status.code(),
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

fn fake_openrouter_key() -> String {
    format!("sk-{}-{}", "or-v1", "abcdef01".repeat(8))
}

fn free_port() -> u16 {
    for _ in 0..80 {
        let port = NEXT_PORT.fetch_add(1, Ordering::SeqCst);
        if port > 3299 {
            NEXT_PORT.store(3270, Ordering::SeqCst);
            continue;
        }
        let addr: SocketAddr = format!("127.0.0.1:{port}")
            .parse()
            .expect("valid loopback addr");
        if TcpListener::bind(addr).is_ok() {
            return port;
        }
    }
    panic!("no free port available in 3270-3299");
}

fn wait_for_listener_pid(port: u16) -> u32 {
    let started = Instant::now();
    while started.elapsed() < WAIT_TIMEOUT {
        if let Some(pid) = listener_pid(port) {
            return pid;
        }
        thread::sleep(Duration::from_millis(50));
    }
    panic!("tool-call endpoint never listened on 127.0.0.1:{port}");
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

fn kill_listener(pid: u32) {
    let _ = Command::new("kill").arg(pid.to_string()).status();
}

fn wait_for_port_released(port: u16) {
    let started = Instant::now();
    while started.elapsed() < WAIT_TIMEOUT {
        if listener_pid(port).is_none() {
            assert!(
                TcpListener::bind(("127.0.0.1", port)).is_ok(),
                "port {port} has no lsof listener but cannot be rebound"
            );
            return;
        }
        thread::sleep(Duration::from_millis(50));
    }
    panic!("tool-call endpoint port {port} was not released after killing its listener PID");
}

const TOOL_CALL_SERVER: &str = r#"
import json
import sys
from http.server import BaseHTTPRequestHandler, HTTPServer

port = int(sys.argv[1])
request_count = 0

class Handler(BaseHTTPRequestHandler):
    def do_POST(self):
        global request_count
        request_count += 1
        length = int(self.headers.get("content-length", "0"))
        self.rfile.read(length)
        if request_count == 1:
            payload = (
                'data: {"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_1","type":"function","function":{"name":"write","arguments":"{\\"path\\":\\"target.txt\\",\\"content\\":\\"BLOCKED\\"}"}}]},"finish_reason":null}]}\n\n'
                'data: {"choices":[{"delta":{},"finish_reason":"tool_calls"}]}\n\n'
                'data: [DONE]\n\n'
            ).encode("utf-8")
        else:
            payload = (
                'data: {"choices":[{"delta":{"content":"done"},"finish_reason":"stop"}]}\n\n'
                'data: [DONE]\n\n'
            ).encode("utf-8")
        self.send_response(200)
        self.send_header("Content-Type", "text/event-stream")
        self.send_header("Cache-Control", "no-cache")
        self.send_header("Content-Length", str(len(payload)))
        self.send_header("Connection", "close")
        self.end_headers()
        self.wfile.write(payload)

    def log_message(self, fmt, *args):
        return

HTTPServer(("127.0.0.1", port), Handler).serve_forever()
"#;
