//! Black-box coverage for plugin request/schema shaping hooks through `cortex run`.
//!
//! The provider below is a real loopback OpenAI-compatible capture endpoint in the mission port
//! range. It records the exact HTTP request body and headers sent by the shipped OpenAI-compatible
//! adapter, then returns a minimal streamed assistant response.

use std::io::Read;
use std::net::{SocketAddr, TcpListener};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::sync::atomic::{AtomicU16, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::Value;

const RUN_TIMEOUT: Duration = Duration::from_secs(60);
const WAIT_TIMEOUT: Duration = Duration::from_secs(10);

static NEXT_PORT: AtomicU16 = AtomicU16::new(3260);

#[derive(Clone, Debug)]
struct CapturedRequest {
    headers: String,
    body: Value,
}

struct Scratch {
    home: PathBuf,
    project: PathBuf,
    db: PathBuf,
}

impl Scratch {
    fn new(tag: &str) -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "cortex-plugin-request-shaping-{tag}-{}-{nanos}",
            std::process::id()
        ));
        let home = root.join("home");
        let project = root.join("project");
        std::fs::create_dir_all(&home).expect("create scratch home");
        std::fs::create_dir_all(&project).expect("create scratch project");
        Self {
            home,
            project,
            db: root.join("data/cortex.db"),
        }
    }
}

struct ProviderCapture {
    port: u16,
    capture_path: PathBuf,
    child: Option<Child>,
    listener_pid: u32,
}

impl ProviderCapture {
    fn spawn(_expected_requests: usize) -> Self {
        let port = free_port();
        let dir = std::env::temp_dir().join(format!(
            "cortex-plugin-capture-server-{}-{}",
            std::process::id(),
            port
        ));
        let _ = std::fs::create_dir_all(&dir);
        let script = dir.join("capture.py");
        let capture_path = dir.join("capture.jsonl");
        std::fs::write(&script, CAPTURE_SERVER).expect("write capture server");
        let child = Command::new("python3")
            .arg(&script)
            .arg(port.to_string())
            .arg(&capture_path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn provider capture endpoint");
        let listener_pid = wait_for_listener_pid(port);
        assert_eq!(
            listener_pid,
            child.id(),
            "lsof should resolve the spawned capture endpoint PID"
        );
        Self {
            port,
            capture_path,
            child: Some(child),
            listener_pid,
        }
    }

    fn base_url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    fn captured_requests(&self) -> Vec<CapturedRequest> {
        let contents = std::fs::read_to_string(&self.capture_path).unwrap_or_default();
        contents
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(|line| {
                let value: Value = serde_json::from_str(line).expect("captured request JSONL");
                CapturedRequest {
                    headers: value["headers"].as_str().unwrap_or_default().to_owned(),
                    body: value["body"].clone(),
                }
            })
            .collect()
    }

    fn shutdown_and_assert_released(mut self) {
        kill_listener(self.listener_pid);
        if let Some(mut child) = self.child.take() {
            let _ = child.wait();
        }
        wait_for_port_released(self.port);
    }
}

impl Drop for ProviderCapture {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            kill_listener(self.listener_pid);
            let _ = child.wait();
        }
    }
}

#[test]
fn plugin_shapes_messages_params_headers_system_and_tool_definitions() {
    let provider = ProviderCapture::spawn(1);
    let scratch = Scratch::new("all-hooks");
    write_shaping_plugin(&scratch);

    let out = run(cortex_in(
        &scratch,
        &provider,
        &["run", "--format", "json", "hello from user"],
    ));
    assert_success(&out);

    let requests = provider.captured_requests();
    assert_eq!(requests.len(), 1, "expected exactly one captured request");
    let capture = &requests[0];

    let messages = capture.body["messages"]
        .as_array()
        .expect("messages array in request");
    let user_message = messages
        .iter()
        .find(|message| message["role"] == "user")
        .expect("user message");
    assert!(
        user_message["content"]
            .as_str()
            .is_some_and(|content| content.contains("VAL_PLUGIN_014_MESSAGE")),
        "chat.message hook should append the sentinel token: {}",
        capture.body
    );
    let system_message = messages
        .iter()
        .find(|message| message["role"] == "system")
        .expect("system message");
    assert!(
        system_message["content"]
            .as_str()
            .is_some_and(|content| content.starts_with("VAL_PLUGIN_024_SYSTEM\n")),
        "system transform hook should prepend the directive: {}",
        capture.body
    );
    assert_eq!(capture.body["temperature"].as_f64(), Some(0.123));
    assert!(
        capture
            .headers
            .to_ascii_lowercase()
            .contains("x-cortex-plugin: m5"),
        "chat.headers hook should inject X-Cortex-Plugin; headers:\n{}",
        capture.headers
    );

    let doc: Value = serde_json::from_slice(&out.stdout).expect("strict JSON document");
    assert!(
        doc["tool_definitions"]
            .as_array()
            .expect("tool definitions")
            .iter()
            .any(|definition| {
                definition["name"] == "read"
                    && definition["description"] == "VAL_PLUGIN_023_READ_DESCRIPTION"
            }),
        "tool.definition hook should override read description: {doc}"
    );
    provider.shutdown_and_assert_released();
}

#[test]
fn plugin_header_is_absent_without_plugin() {
    let provider = ProviderCapture::spawn(1);
    let scratch = Scratch::new("no-plugin");

    let out = run(cortex_in(
        &scratch,
        &provider,
        &["run", "--format", "json", "hello without plugin"],
    ));
    assert_success(&out);

    let requests = provider.captured_requests();
    assert_eq!(requests.len(), 1, "expected exactly one captured request");
    assert!(
        !requests[0]
            .headers
            .to_ascii_lowercase()
            .contains("x-cortex-plugin:"),
        "plugin header must be absent when no plugin is configured; headers:\n{}",
        requests[0].headers
    );
    provider.shutdown_and_assert_released();
}

#[test]
fn multiple_plugins_apply_chat_message_in_config_order_across_runs() {
    let provider = ProviderCapture::spawn(2);
    let scratch = Scratch::new("ordering");
    let log = scratch.project.join("order.log");
    write_order_plugin(&scratch.project.join("a.mjs"), "a");
    write_order_plugin(&scratch.project.join("b.mjs"), "b");
    std::fs::write(
        scratch.project.join("cortex.toml"),
        format!(
            "plugin = [[\"./a.mjs\", {{ log = \"{}\" }}], [\"./b.mjs\", {{ log = \"{}\" }}]]\n",
            log.display(),
            log.display()
        ),
    )
    .expect("write plugin config");

    for prompt in ["ordering run one", "ordering run two"] {
        let out = run(cortex_in(&scratch, &provider, &["run", prompt]));
        assert_success(&out);
    }

    let entries = std::fs::read_to_string(&log).expect("order log");
    let lines = entries.lines().collect::<Vec<_>>();
    assert_eq!(
        lines,
        vec!["a", "b", "a", "b"],
        "plugins should run same-hook callbacks in config order on every run"
    );
    provider.shutdown_and_assert_released();
}

fn write_shaping_plugin(scratch: &Scratch) {
    std::fs::write(
        scratch.project.join("plugin.mjs"),
        r#"
export default async function plugin(input, options) {
  return {
    "chat.message": async (message) => {
      if (message.role === "user") {
        return { ...message, content: `${message.content} VAL_PLUGIN_014_MESSAGE` };
      }
      return message;
    },
    "chat.params": async (params) => {
      return { ...params, temperature: 0.123 };
    },
    "chat.headers": async (headers) => {
      return { ...headers, "X-Cortex-Plugin": "m5" };
    },
    "tool.definition": async (definition) => {
      if (definition.name === "read") {
        return { ...definition, description: "VAL_PLUGIN_023_READ_DESCRIPTION" };
      }
      return definition;
    },
    "experimental.chat.system.transform": async (system) => {
      return `VAL_PLUGIN_024_SYSTEM\n${system}`;
    }
  };
}
"#,
    )
    .expect("write shaping plugin");
    std::fs::write(
        scratch.project.join("cortex.toml"),
        "plugin = [\"./plugin.mjs\"]\n",
    )
    .expect("write plugin config");
}

fn write_order_plugin(path: &Path, id: &str) {
    std::fs::write(
        path,
        format!(
            r#"
import {{ appendFile }} from "node:fs/promises";

export default async function plugin(input, options) {{
  return {{
    "chat.message": async (message) => {{
      if (message.role === "user") {{
        await appendFile(options.log, "{id}\n");
      }}
      return message;
    }}
  }};
}}
"#
        ),
    )
    .expect("write order plugin");
}

fn cortex_in(scratch: &Scratch, provider: &ProviderCapture, args: &[&str]) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_cortex"));
    command.current_dir(&scratch.project);
    command.args(args);
    command.env("HOME", &scratch.home);
    command.env("CORTEX_DB_PATH", &scratch.db);
    command.env("OPENROUTER_API_KEY", fake_openrouter_key());
    command.env("CORTEX_OPENROUTER_BASE_URL", provider.base_url());
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
        "cortex run failed with status {:?}\nstdout:\n{}\nstderr:\n{}",
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
            NEXT_PORT.store(3260, Ordering::SeqCst);
            continue;
        }
        let addr: SocketAddr = format!("127.0.0.1:{port}")
            .parse()
            .expect("valid loopback addr");
        if TcpListener::bind(addr).is_ok() {
            return port;
        }
    }
    panic!("no free port available in 3260-3299");
}

fn wait_for_listener_pid(port: u16) -> u32 {
    let started = Instant::now();
    while started.elapsed() < WAIT_TIMEOUT {
        if let Some(pid) = listener_pid(port) {
            return pid;
        }
        thread::sleep(Duration::from_millis(50));
    }
    panic!("capture endpoint never listened on 127.0.0.1:{port}");
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
    panic!("capture endpoint port {port} was not released after killing its lsof-resolved PID");
}

const CAPTURE_SERVER: &str = r#"
import json
import sys
from http.server import BaseHTTPRequestHandler, HTTPServer

port = int(sys.argv[1])
capture_path = sys.argv[2]

class Handler(BaseHTTPRequestHandler):
    def do_POST(self):
        length = int(self.headers.get("content-length", "0"))
        raw = self.rfile.read(length)
        with open(capture_path, "a", encoding="utf-8") as fh:
            json.dump({
                "headers": str(self.headers),
                "body": json.loads(raw.decode("utf-8")),
            }, fh)
            fh.write("\n")
            fh.flush()
        payload = (
            'data: {"choices":[{"delta":{"content":"plugin request shaping ok"},"finish_reason":"stop"}]}\n\n'
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
