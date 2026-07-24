use std::fs;
use std::io::Read;
use std::net::{SocketAddr, TcpListener};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::sync::atomic::{AtomicU16, Ordering};
use std::thread;
use std::time::{Duration, Instant};

const RUN_TIMEOUT: Duration = Duration::from_secs(60);
const WAIT_TIMEOUT: Duration = Duration::from_secs(10);

static NEXT_PORT: AtomicU16 = AtomicU16::new(18765);

struct Scratch {
    root: PathBuf,
    home: PathBuf,
    project: PathBuf,
    db: PathBuf,
    config_log: PathBuf,
}

impl Scratch {
    fn new(tag: &str) -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "cortex-gap-plugin-config-workspace-{tag}-{}-{nanos}",
            std::process::id()
        ));
        let home = root.join("home");
        let project = root.join("project");
        fs::create_dir_all(&home).expect("create scratch home");
        fs::create_dir_all(&project).expect("create scratch project");
        Self {
            root: root.clone(),
            home,
            project: project.clone(),
            db: root.join("data/cortex.db"),
            config_log: project.join("config-hook.jsonl"),
        }
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
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
            "cortex-gap-plugin-config-workspace-provider-{}-{port}",
            std::process::id()
        ));
        let _ = fs::create_dir_all(&dir);
        let script = dir.join("server.py");
        fs::write(&script, TOOL_CALL_SERVER).expect("write tool-call server");
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
fn config_hook_gets_effective_config_and_workspace_registration_is_listed() {
    let provider = ToolCallProvider::spawn();
    let scratch = Scratch::new("plugin004-015");
    write_plugin(&scratch.project, &scratch.config_log);

    let out = run(cortex_in(
        &scratch,
        &provider,
        &[
            "run",
            "--format",
            "json",
            "please run the bash command echo blocked-by-config-hook",
        ],
    ));
    assert_success(&out);

    let run_json: serde_json::Value = serde_json::from_slice(&out.stdout).expect("run json");
    assert!(
        run_json["tool_results"]
            .as_array()
            .is_some_and(|results| results.iter().any(|result| {
                result["name"] == "bash"
                    && result["is_error"] == true
                    && result["parts"]
                        .to_string()
                        .to_lowercase()
                        .contains("permission denied")
            })),
        "plugin config hook permission rule should be consumed by the run: {run_json}"
    );
    assert!(
        run_json["workspace_adapters"]
            .as_array()
            .is_some_and(|entries| entries.iter().any(|entry| {
                entry["type"] == "folder"
                    && entry["name"] == "Folder adapter"
                    && entry["description"] == "Workspace from plugin config test"
            })),
        "workspace registration should be surfaced in machine output: {run_json}"
    );

    let log = fs::read_to_string(&scratch.config_log).expect("config hook log");
    let observed: serde_json::Value = serde_json::from_str(
        log.lines()
            .find(|line| !line.trim().is_empty())
            .expect("one config hook line"),
    )
    .expect("config hook json");
    assert_eq!(
        observed["model"], "anthropic/claude-sonnet-4",
        "config hook should observe env-overridden model"
    );
    assert_eq!(
        observed["agent"], "build",
        "config hook should observe resolved file-layer agent"
    );
    assert_eq!(
        observed["max_steps"], 25,
        "config hook should observe defaults too"
    );

    provider.shutdown_and_assert_released();
}

fn write_plugin(project: &Path, config_log: &Path) {
    fs::write(
        project.join("plugin.mjs"),
        r#"
import { appendFile } from "node:fs/promises";

export default async function plugin(input, options) {
  input.experimental_workspace.register("folder", {
    name: "Folder adapter",
    description: "Workspace from plugin config test"
  });

  return {
    async config(config) {
      await appendFile(options.log, JSON.stringify({
        model: config.model ?? null,
        agent: config.agent ?? null,
        max_steps: config.max_steps ?? null
      }) + "\n");
      return {
        permission: [
          { tool: "bash", pattern: "echo blocked-by-config-hook", action: "deny" }
        ]
      };
    }
  };
}
"#,
    )
    .expect("write plugin");
    fs::write(
        project.join("cortex.toml"),
        format!(
            r#"
model = "google/gemini-2.5-flash"
agent = "build"
plugin = [[ "./plugin.mjs", {{ log = "{}" }} ]]
"#,
            config_log.display()
        ),
    )
    .expect("write config");
}

fn cortex_in(scratch: &Scratch, provider: &ToolCallProvider, args: &[&str]) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_cortex"));
    command.current_dir(&scratch.project);
    command.args(args);
    command.env("HOME", &scratch.home);
    command.env("CORTEX_DB_PATH", &scratch.db);
    command.env("OPENROUTER_API_KEY", fake_openrouter_key());
    command.env("CORTEX_OPENROUTER_BASE_URL", provider.base_url());
    command.env("CORTEX_MODEL", "anthropic/claude-sonnet-4");
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
        if port > 18864 {
            NEXT_PORT.store(18835, Ordering::SeqCst);
            continue;
        }
        let addr: SocketAddr = format!("127.0.0.1:{port}")
            .parse()
            .expect("valid loopback addr");
        if TcpListener::bind(addr).is_ok() {
            return port;
        }
    }
    panic!("no free port available in 18765-18864");
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
                'data: {"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_1","type":"function","function":{"name":"bash","arguments":"{\\"command\\":\\"echo blocked-by-config-hook\\"}"}}]},"finish_reason":null}]}\n\n'
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
