use std::io::Read;
use std::io::Write;
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use std::time::{Duration, Instant};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

struct Sandbox {
    home: PathBuf,
    project: PathBuf,
}

impl Sandbox {
    fn new(tag: &str) -> Self {
        let home = std::env::temp_dir().join(format!(
            "cortex-cli-plugin-registration-{tag}-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let project = home.join("project");
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(&project).expect("create project");
        Self { home, project }
    }
}

fn skip_without_runtime() -> bool {
    if plugins::detect_runtime().is_none() {
        eprintln!("skipping plugin CLI registration proof: no Bun, Node, or Deno on PATH");
        true
    } else {
        false
    }
}

fn write_plugin(path: &Path) {
    std::fs::write(
        path,
        r#"
export default async function plugin(input, options) {
  return {
    async tool() {
      return {
        name: "cortex_echo",
        description: "Echo text from a plugin tool.",
        parameters: {
          type: "object",
          properties: { text: { type: "string" } },
          required: ["text"],
          additionalProperties: false
        },
        async execute(args) {
          return `ECHO:${args.text}`;
        }
      };
    },
    async auth() {
      return [{ id: "plugin-auth", name: "Plugin Auth", env: ["PLUGIN_AUTH_TOKEN"] }];
    },
    async provider() {
      return {
        id: "plugin-provider",
        name: "Plugin Provider",
        models: [
          { id: "plugin/example-model", name: "Plugin Example", tool_call: false }
        ],
        async complete(request) {
          return `PLUGIN_PROVIDER:${request.model}:${request.messages.length}`;
        }
      };
    }
  };
}
"#,
    )
    .expect("write plugin");
}

fn cortex(sandbox: &Sandbox, args: &[&str]) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_cortex"));
    command.current_dir(&sandbox.project);
    command.args(args);
    command.env("HOME", &sandbox.home);
    command.env("XDG_CONFIG_HOME", sandbox.home.join(".config"));
    command.env("CORTEX_DATA_DIR", sandbox.home.join(".local/share/cortex"));
    command.env_remove("OPENROUTER_API_KEY");
    command.env_remove("CORTEX_OPENROUTER_BASE_URL");
    command.env_remove("CORTEX_NO_NET");
    command.env_remove("CORTEX_PURE");
    command
}

fn run(mut command: Command, timeout: Duration) -> Output {
    command.stdin(Stdio::null());
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());

    let mut child = command.spawn().expect("spawn cortex binary");
    let mut stdout = child.stdout.take().expect("stdout");
    let mut stderr = child.stderr.take().expect("stderr");
    let stdout_thread = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = stdout.read_to_end(&mut bytes);
        bytes
    });
    let stderr_thread = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = stderr.read_to_end(&mut bytes);
        bytes
    });

    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().expect("try wait") {
            break status;
        }
        if started.elapsed() > timeout {
            let _ = child.kill();
            let _ = child.wait();
            panic!("cortex command did not terminate within {timeout:?}");
        }
        std::thread::sleep(Duration::from_millis(20));
    };

    Output {
        status,
        stdout: stdout_thread.join().expect("join stdout"),
        stderr: stderr_thread.join().expect("join stderr"),
    }
}

struct ProviderFixture {
    base_url: String,
    handle: thread::JoinHandle<()>,
}

impl ProviderFixture {
    fn start() -> Self {
        let listener = bind_fixture_listener();
        let port = listener.local_addr().expect("fixture address").port();
        let handle = thread::spawn(move || {
            listener
                .set_nonblocking(true)
                .expect("set fixture listener nonblocking");
            let deadline = Instant::now() + Duration::from_secs(10);
            let mut served = 0;
            let mut last_activity = Instant::now();
            while Instant::now() < deadline {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        read_http_request(&mut stream).expect("read provider request");
                        write_sse_response(&mut stream).expect("write provider response");
                        served += 1;
                        last_activity = Instant::now();
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        if served > 0 && last_activity.elapsed() > Duration::from_secs(2) {
                            return;
                        }
                        thread::sleep(Duration::from_millis(10));
                    }
                    Err(error) => panic!("accept fixture connection: {error}"),
                }
            }
            assert!(served > 0, "fixture did not receive a provider request");
        });
        Self {
            base_url: format!("http://127.0.0.1:{port}"),
            handle,
        }
    }

    fn finish(self) {
        self.handle.join().expect("join provider fixture");
    }
}

fn bind_fixture_listener() -> TcpListener {
    for port in (server::MIN_PORT..=server::MAX_PORT).rev() {
        if let Ok(listener) = TcpListener::bind(("127.0.0.1", port)) {
            return listener;
        }
    }
    panic!(
        "no available fixture port in the reserved Cortex range {}-{}",
        server::MIN_PORT,
        server::MAX_PORT
    );
}

fn read_http_request(stream: &mut TcpStream) -> std::io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    let mut bytes = Vec::new();
    let mut chunk = [0_u8; 1024];
    let header_end = loop {
        let read = stream.read(&mut chunk)?;
        if read == 0 {
            return Ok(());
        }
        bytes.extend_from_slice(&chunk[..read]);
        if let Some(position) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
            break position + 4;
        }
    };
    let headers = String::from_utf8_lossy(&bytes[..header_end]);
    let content_length = headers
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().ok())
                .flatten()
        })
        .unwrap_or(0);
    let mut remaining = content_length.saturating_sub(bytes.len().saturating_sub(header_end));
    while remaining > 0 {
        let read = stream.read(&mut chunk)?;
        if read == 0 {
            break;
        }
        remaining = remaining.saturating_sub(read);
    }
    Ok(())
}

fn write_sse_response(stream: &mut TcpStream) -> std::io::Result<()> {
    let body = concat!(
        "data: {\"id\":\"plugin-pure-fixture\",\"object\":\"chat.completion.chunk\",",
        "\"model\":\"google/gemini-2.5-flash\",\"choices\":[{\"index\":0,",
        "\"delta\":{\"content\":\"PLUGIN_PURE_OK\"},\"finish_reason\":null}]}\n\n",
        "data: {\"id\":\"plugin-pure-fixture\",\"object\":\"chat.completion.chunk\",",
        "\"model\":\"google/gemini-2.5-flash\",\"choices\":[{\"index\":0,",
        "\"delta\":{},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":1,",
        "\"completion_tokens\":1,\"total_tokens\":2}}\n\n",
        "data: [DONE]\n\n"
    );
    let mut response = String::new();
    response.push_str("HTTP/1.1 200 OK\r\n");
    response.push_str("content-type: text/event-stream\r\n");
    response.push_str("cache-control: no-cache\r\n");
    response.push_str("connection: close\r\n");
    response.push_str(&format!("content-length: {}\r\n\r\n", body.len()));
    response.push_str(body);
    stream.write_all(response.as_bytes())?;
    stream.flush()
}

#[test]
fn plugin_models_and_provider_surfaces_are_discoverable() {
    if skip_without_runtime() {
        return;
    }

    let sandbox = Sandbox::new("surfaces");
    write_plugin(&sandbox.project.join("plugin.mjs"));
    std::fs::write(
        sandbox.project.join("cortex.toml"),
        "plugin = [\"./plugin.mjs\"]\n",
    )
    .expect("write config");

    let models = run(
        cortex(&sandbox, &["models", "--format", "json"]),
        Duration::from_secs(30),
    );
    assert_eq!(models.status.code(), Some(0));
    let models_json: serde_json::Value =
        serde_json::from_slice(&models.stdout).expect("models json parses");
    assert!(
        models_json["providers"]
            .as_array()
            .unwrap()
            .iter()
            .any(|provider| provider["id"] == "plugin-provider"
                && provider["models"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|model| model["id"] == "plugin/example-model")),
        "plugin model should be listed: {models_json}"
    );

    let run_output = run(
        cortex(
            &sandbox,
            &[
                "run",
                "--model",
                "plugin/example-model",
                "--format",
                "json",
                "select the plugin provider",
            ],
        ),
        Duration::from_secs(30),
    );
    assert_eq!(
        run_output.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&run_output.stderr)
    );
    let run_json: serde_json::Value =
        serde_json::from_slice(&run_output.stdout).expect("run json parses");
    assert!(
        run_json["content"]
            .as_str()
            .unwrap_or_default()
            .contains("PLUGIN_PROVIDER:plugin/example-model"),
        "plugin provider JS body should produce the answer: {run_json}"
    );
    assert!(
        run_json["tools"]
            .as_array()
            .unwrap()
            .iter()
            .any(|tool| tool == "cortex_echo"),
        "plugin tool should be advertised in the turn: {run_json}"
    );
}

#[test]
fn plugin_registered_surfaces_are_absent_without_the_plugin() {
    let sandbox = Sandbox::new("absent");

    let models = run(
        cortex(&sandbox, &["models", "--format", "json"]),
        Duration::from_secs(10),
    );
    assert_eq!(models.status.code(), Some(0));
    let models_json: serde_json::Value =
        serde_json::from_slice(&models.stdout).expect("models json parses");
    assert!(
        !models_json["providers"]
            .as_array()
            .unwrap()
            .iter()
            .any(|provider| provider["id"] == "plugin-provider")
    );
}

#[test]
fn pure_removes_plugin_tools_but_preserves_builtin_tool_advertisements() {
    if skip_without_runtime() {
        return;
    }

    let sandbox = Sandbox::new("pure-tools");
    write_plugin(&sandbox.project.join("plugin.mjs"));
    std::fs::write(
        sandbox.project.join("cortex.toml"),
        "plugin = [\"./plugin.mjs\"]\n",
    )
    .expect("write plugin config");

    let normal_fixture = ProviderFixture::start();
    let mut normal = cortex(
        &sandbox,
        &[
            "run",
            "--format",
            "json",
            "Reply with exactly PLUGIN_PURE_OK.",
        ],
    );
    normal
        .env("OPENROUTER_API_KEY", "test-openrouter-key")
        .env("CORTEX_OPENROUTER_BASE_URL", &normal_fixture.base_url);
    let normal = run(normal, Duration::from_secs(30));
    normal_fixture.finish();
    assert_eq!(
        normal.status.code(),
        Some(0),
        "normal run failed: {}",
        String::from_utf8_lossy(&normal.stderr)
    );
    let normal_doc: serde_json::Value =
        serde_json::from_slice(&normal.stdout).expect("normal run JSON");
    assert!(
        normal_doc["tools"]
            .as_array()
            .expect("normal tools")
            .iter()
            .any(|tool| tool == "cortex_echo"),
        "normal tool list must advertise the plugin tool: {normal_doc}"
    );

    let pure_fixture = ProviderFixture::start();
    let mut pure = cortex(
        &sandbox,
        &[
            "run",
            "--pure",
            "--format",
            "json",
            "Reply with exactly PLUGIN_PURE_OK.",
        ],
    );
    pure.env("OPENROUTER_API_KEY", "test-openrouter-key")
        .env("CORTEX_OPENROUTER_BASE_URL", &pure_fixture.base_url);
    let pure = run(pure, Duration::from_secs(30));
    pure_fixture.finish();
    assert_eq!(
        pure.status.code(),
        Some(0),
        "pure run failed: {}",
        String::from_utf8_lossy(&pure.stderr)
    );
    let pure_doc: serde_json::Value = serde_json::from_slice(&pure.stdout).expect("pure run JSON");
    let pure_tools = pure_doc["tools"].as_array().expect("pure tools");
    assert!(
        !pure_tools.iter().any(|tool| tool == "cortex_echo"),
        "--pure must not advertise plugin tools: {pure_doc}"
    );
    for builtin in ["read", "write", "bash"] {
        assert!(
            pure_tools.iter().any(|tool| tool == builtin),
            "--pure must retain built-in `{builtin}`: {pure_doc}"
        );
    }
}
