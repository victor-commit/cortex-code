use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use serde_json::Value;

const RUN_TIMEOUT: Duration = Duration::from_secs(30);

struct Sandbox {
    home: PathBuf,
    project: PathBuf,
    data_dir: PathBuf,
}

impl Sandbox {
    fn new(tag: &str) -> Self {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "cortex-gap-plugin-auth-{tag}-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let project = root.join("project");
        let data_dir = root.join("data");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&project).expect("create project");
        fs::create_dir_all(root.join(".config")).expect("create config dir");
        fs::create_dir_all(root.join(".local/share")).expect("create xdg dir");
        Self {
            home: root,
            project,
            data_dir,
        }
    }

    fn command(&self, args: &[&str]) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_cortex"));
        command.current_dir(&self.project);
        command.args(args);
        command.env("HOME", &self.home);
        command.env("XDG_CONFIG_HOME", self.home.join(".config"));
        command.env("XDG_DATA_HOME", self.home.join(".local/share"));
        command.env("CORTEX_DATA_DIR", &self.data_dir);
        command
            .env_remove("OPENROUTER_API_KEY")
            .env_remove("CORTEX_AUTH_FILE")
            .env_remove("CORTEX_NO_NET")
            .env_remove("RUST_LOG")
            .env_remove("CORTEX_VERBOSE")
            .env_remove("NO_COLOR");
        command
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.home);
    }
}

fn run(mut command: Command) -> Output {
    command.stdin(Stdio::null());
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());

    let mut child = command.spawn().expect("spawn cortex");
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
        if started.elapsed() > RUN_TIMEOUT {
            let _ = child.kill();
            let _ = child.wait();
            panic!("command timed out after {RUN_TIMEOUT:?}");
        }
        std::thread::sleep(Duration::from_millis(20));
    };

    Output {
        status,
        stdout: stdout_thread.join().expect("join stdout"),
        stderr: stderr_thread.join().expect("join stderr"),
    }
}

fn skip_without_runtime() -> bool {
    if plugins::detect_runtime().is_none() {
        eprintln!("skipping plugin auth gap test: no Bun, Node, or Deno on PATH");
        true
    } else {
        false
    }
}

#[test]
fn plugin_auth_methods_flow_through_auth_login_authorize_callback_and_provider_loader() {
    if skip_without_runtime() {
        return;
    }
    let sandbox = Sandbox::new("plugin-auth-flow");
    let marker = sandbox.project.join("provider-auth.json");
    let plugin = sandbox.project.join("plugin.mjs");
    fs::write(
        &plugin,
        r#"
import { writeFile } from "node:fs/promises";

export default async function plugin(input, options) {
  return {
    auth: {
      provider: "plugin-provider",
      name: "Plugin Provider Auth",
      loader: async (auth) => {
        const value = await auth();
        if (value?.type === "api") {
          return {
            kind: "api",
            token: value.key ?? "",
            workspace: value.metadata?.workspace ?? ""
          };
        }
        if (value?.type === "oauth") {
          return {
            kind: "oauth",
            access: value.access ?? "",
            refresh: value.refresh ?? ""
          };
        }
        return value ?? null;
      },
      methods: [
        {
          id: "api",
          type: "api",
          label: "API key",
          prompts: [
            { type: "text", key: "api_key", message: "API key" },
            { type: "text", key: "workspace", message: "Workspace" }
          ],
          async authorize(inputs) {
            return {
              type: "success",
              credential: {
                type: "api",
                key: String(inputs?.api_key ?? ""),
                metadata: { workspace: String(inputs?.workspace ?? "") }
              }
            };
          }
        },
        {
          id: "oauth",
          type: "oauth",
          label: "OAuth code flow",
          async authorize() {
            return {
              method: "code",
              url: "https://example.com/oauth/start",
              instructions: "Enter the provided code.",
              callback(code) {
                return {
                  type: "success",
                  credential: {
                    type: "oauth",
                    access: `access-${code}`,
                    refresh: `refresh-${code}`,
                    expires: 999999999999
                  }
                };
              }
            };
          }
        }
      ]
    },
    async provider() {
      return {
        id: "plugin-provider",
        name: "Plugin Provider",
        models: [{ id: "plugin/auth-model", name: "Plugin Auth Model", tool_call: false }],
        async complete(request) {
          await writeFile(options.marker, JSON.stringify(request.auth ?? null));
          if (request.auth?.kind === "api") {
            return `AUTH_API:${request.auth.workspace}:present`;
          }
          if (request.auth?.kind === "oauth") {
            return `AUTH_OAUTH:${request.auth.access}`;
          }
          return "AUTH_MISSING";
        }
      };
    }
  };
}
"#,
    )
    .expect("write plugin");
    fs::write(
        sandbox.project.join("cortex.toml"),
        format!(
            "plugin = [[\"{}\", {{ marker = \"{}\" }}]]\n",
            plugin.display(),
            marker.display()
        ),
    )
    .expect("write config");

    let list = run(sandbox.command(&["auth", "list", "--format", "json"]));
    assert_eq!(list.status.code(), Some(0), "auth list should succeed");
    let list_doc: Value = serde_json::from_slice(&list.stdout).expect("auth list json");
    assert!(
        list_doc["methods"]
            .as_array()
            .expect("methods array")
            .iter()
            .any(|method| {
                method["provider"] == "plugin-provider"
                    && method["method"] == "api"
                    && method["source"] == "plugin"
            }),
        "plugin auth API method should be listed: {list_doc}"
    );
    assert!(
        list_doc["methods"]
            .as_array()
            .expect("methods array")
            .iter()
            .any(|method| {
                method["provider"] == "plugin-provider"
                    && method["method"] == "oauth"
                    && method["type"] == "oauth"
            }),
        "plugin OAuth method should be listed: {list_doc}"
    );

    let api_secret = "plugin-secret-123";
    let api_login = run(sandbox.command(&[
        "auth",
        "login",
        "plugin-provider",
        "--auth-method",
        "api",
        "--auth-input",
        &format!("api_key={api_secret}"),
        "--auth-input",
        "workspace=acme",
        "--format",
        "json",
    ]));
    assert_eq!(
        api_login.status.code(),
        Some(0),
        "plugin API login should succeed"
    );
    let api_stdout = String::from_utf8_lossy(&api_login.stdout);
    let api_stderr = String::from_utf8_lossy(&api_login.stderr);
    assert!(
        !api_stdout.contains(api_secret) && !api_stderr.contains(api_secret),
        "auth login must not leak plugin API secret"
    );

    let api_run = run(sandbox.command(&[
        "run",
        "--model",
        "plugin/auth-model",
        "--format",
        "json",
        "verify plugin api auth",
    ]));
    assert_eq!(
        api_run.status.code(),
        Some(0),
        "plugin API run should succeed"
    );
    let api_run_doc: Value = serde_json::from_slice(&api_run.stdout).expect("run json");
    assert_eq!(api_run_doc["content"], "AUTH_API:acme:present");
    let marker_value: Value =
        serde_json::from_slice(&fs::read(marker.as_path()).expect("marker")).expect("marker json");
    assert_eq!(marker_value["kind"], "api");
    assert_eq!(marker_value["workspace"], "acme");
    assert_eq!(marker_value["token"], "plugin-secret-123");

    let oauth_login = run(sandbox.command(&[
        "auth",
        "login",
        "plugin-provider",
        "--auth-method",
        "oauth",
        "--oauth-code",
        "code-777",
        "--format",
        "json",
    ]));
    assert_eq!(
        oauth_login.status.code(),
        Some(0),
        "plugin OAuth login should succeed"
    );
    let oauth_run = run(sandbox.command(&[
        "run",
        "--model",
        "plugin/auth-model",
        "--format",
        "json",
        "verify plugin oauth auth",
    ]));
    assert_eq!(
        oauth_run.status.code(),
        Some(0),
        "plugin OAuth run should succeed"
    );
    let oauth_doc: Value = serde_json::from_slice(&oauth_run.stdout).expect("oauth run json");
    assert_eq!(oauth_doc["content"], "AUTH_OAUTH:access-code-777");
}

#[test]
fn plugin_tool_auth_required_fails_closed_without_stored_credential() {
    if skip_without_runtime() {
        return;
    }

    let sandbox = Sandbox::new("plugin-tool-auth-required");
    let marker = sandbox.project.join("tool-auth-marker.json");
    write_tool_auth_plugin(&sandbox, &marker, true);

    let fixture = start_tool_call_fixture("plugin_auth_probe");
    let mut run_cmd = sandbox.command(&["run", "--format", "json", "probe plugin tool auth"]);
    run_cmd.env("OPENROUTER_API_KEY", fake_openrouter_key());
    run_cmd.env("CORTEX_OPENROUTER_BASE_URL", fixture.base_url());
    let run_out = run(run_cmd);
    fixture.join();
    assert_eq!(
        run_out.status.code(),
        Some(0),
        "run should complete through the fixture: stderr={}",
        String::from_utf8_lossy(&run_out.stderr)
    );
    let run_doc: Value = serde_json::from_slice(&run_out.stdout).expect("run json");
    let content = run_doc["content"].as_str().unwrap_or_default();
    assert!(
        content.contains("missing required auth credential for provider `plugin-tool-auth`"),
        "tool auth should fail closed with a clear diagnostic, got: {run_doc}"
    );
    assert!(
        !marker.exists(),
        "required auth failure should stop before plugin JS tool execution"
    );
}

#[test]
fn plugin_tool_execution_receives_auth_payload_after_auth_login_without_secret_leaks() {
    if skip_without_runtime() {
        return;
    }

    let sandbox = Sandbox::new("plugin-tool-auth-present");
    let marker = sandbox.project.join("tool-auth-marker.json");
    let audit_log = sandbox.project.join("audit.jsonl");
    write_tool_auth_plugin(&sandbox, &marker, false);

    let secret = "plugin-tool-secret-789";
    let login = run(sandbox.command(&[
        "auth",
        "login",
        "plugin-tool-auth",
        "--auth-method",
        "api",
        "--auth-input",
        &format!("api_key={secret}"),
        "--format",
        "json",
    ]));
    assert_eq!(
        login.status.code(),
        Some(0),
        "plugin auth login should succeed"
    );
    let login_output = format!(
        "{}{}",
        String::from_utf8_lossy(&login.stdout),
        String::from_utf8_lossy(&login.stderr)
    );
    assert!(
        !login_output.contains(secret),
        "auth login output must not leak the plugin secret"
    );

    let fixture = start_tool_call_fixture("plugin_auth_probe");
    let mut run_cmd = sandbox.command(&["run", "--format", "json", "probe plugin tool auth"]);
    run_cmd.env("OPENROUTER_API_KEY", fake_openrouter_key());
    run_cmd.env("CORTEX_OPENROUTER_BASE_URL", fixture.base_url());
    run_cmd.env("CORTEX_AUDIT_LOG", &audit_log);
    let run_out = run(run_cmd);
    fixture.join();
    assert_eq!(
        run_out.status.code(),
        Some(0),
        "run should succeed with stored plugin auth: stderr={}",
        String::from_utf8_lossy(&run_out.stderr)
    );
    let run_doc: Value = serde_json::from_slice(&run_out.stdout).expect("run json");
    assert_eq!(run_doc["content"], "AUTH_MARKER_PRESENT");
    let marker_doc: Value = serde_json::from_slice(&fs::read(&marker).expect("tool marker file"))
        .expect("tool marker json");
    assert_eq!(marker_doc["marker"], "AUTH_MARKER_PRESENT");

    let run_output = format!(
        "{}{}",
        String::from_utf8_lossy(&run_out.stdout),
        String::from_utf8_lossy(&run_out.stderr)
    );
    assert!(
        !run_output.contains(secret),
        "run output must not leak the plugin secret"
    );
    let audit_output = fs::read_to_string(&audit_log).unwrap_or_default();
    assert!(
        !audit_output.contains(secret),
        "audit output must not leak the plugin secret"
    );
}

#[test]
fn built_in_auth_integrations_are_listed_and_xai_key_authenticates_provider_runs_without_leaks() {
    let sandbox = Sandbox::new("builtin-auth");
    let model_id = "xai/test-model";
    let api_key = "xai-secret-abc";
    let fixture = start_provider_fixture(model_id, api_key);
    sandbox_write_config(&sandbox, model_id, fixture.base_url());

    let list = run(sandbox.command(&["auth", "list", "--format", "json"]));
    assert_eq!(list.status.code(), Some(0), "auth list should succeed");
    let list_doc: Value = serde_json::from_slice(&list.stdout).expect("auth list json");
    let methods = list_doc["methods"].as_array().expect("methods array");
    for provider in ["copilot", "codex", "gitlab", "azure", "xai"] {
        assert!(
            methods.iter().any(|method| method["provider"] == provider),
            "built-in auth method `{provider}` should be listed: {list_doc}"
        );
    }

    let login = run(sandbox.command(&[
        "auth",
        "login",
        "xai",
        "--api-key",
        api_key,
        "--format",
        "json",
    ]));
    assert_eq!(login.status.code(), Some(0), "xai login should succeed");
    let login_out = format!(
        "{}{}",
        String::from_utf8_lossy(&login.stdout),
        String::from_utf8_lossy(&login.stderr)
    );
    assert!(
        !login_out.contains(api_key),
        "auth login must not leak xai secret"
    );

    let run_out = run(sandbox.command(&["run", "--format", "json", "xai fixture prompt"]));
    fixture.join();
    assert_eq!(
        run_out.status.code(),
        Some(0),
        "run should succeed with stored xai key: stderr={}",
        String::from_utf8_lossy(&run_out.stderr)
    );
    let run_doc: Value = serde_json::from_slice(&run_out.stdout).expect("run json");
    assert_eq!(run_doc["model"], model_id);
    assert!(
        run_doc["content"]
            .as_str()
            .is_some_and(|content| content.contains("fixture-ok")),
        "fixture response should be returned: {run_doc}"
    );
    let run_output = format!(
        "{}{}",
        String::from_utf8_lossy(&run_out.stdout),
        String::from_utf8_lossy(&run_out.stderr)
    );
    assert!(
        !run_output.contains(api_key),
        "run output must not leak xai secret"
    );
}

fn sandbox_write_config(sandbox: &Sandbox, model_id: &str, base_url: &str) {
    fs::write(
        sandbox.project.join("cortex.toml"),
        format!(
            r#"
[provider]
enabled = ["xai"]

[provider.model_overrides]
"google/gemini-2.5-flash" = "{model_id}"

[provider.xai]
base_url = "{base_url}"
models = [{{ id = "{model_id}", name = "xAI Fixture Model", tool_call = true, context_length = 4096, max_output_tokens = 512 }}]
"#
        ),
    )
    .expect("write config");
}

fn write_tool_auth_plugin(sandbox: &Sandbox, marker: &std::path::Path, require_auth: bool) {
    let tool_auth = if require_auth {
        r#"auth: { provider: "plugin-tool-auth", required: true },"#
    } else {
        ""
    };
    fs::write(
        sandbox.project.join("plugin.mjs"),
        format!(
            r#"
import {{ writeFile }} from "node:fs/promises";

export default async function plugin() {{
  return {{
    auth: {{
      provider: "plugin-tool-auth",
      name: "Plugin Tool Auth",
      loader: async (loadAuth) => {{
        const value = await loadAuth();
        if (value?.type === "api" && typeof value.key === "string" && value.key.length > 0) {{
          return {{ marker: "AUTH_MARKER_PRESENT" }};
        }}
        return {{ marker: "AUTH_MARKER_MISSING" }};
      }},
      methods: [{{
        id: "api",
        type: "api",
        label: "API key",
        prompts: [{{ type: "text", key: "api_key", message: "API key" }}],
        async authorize(inputs) {{
          return {{
            type: "success",
            credential: {{
              type: "api",
              key: String(inputs?.api_key ?? "")
            }}
          }};
        }}
      }}]
    }},
    async tool() {{
      return {{
        name: "plugin_auth_probe",
        description: "Return whether auth payload reached the plugin tool body.",
        {tool_auth}
        parameters: {{
          type: "object",
          properties: {{}},
          additionalProperties: false
        }},
        async execute(_args, ctx) {{
          const marker = String(ctx?.auth?.marker ?? "AUTH_MARKER_MISSING");
          await writeFile("{marker_path}", JSON.stringify({{ marker }}));
          return marker;
        }}
      }};
    }}
  }};
}}
"#,
            marker_path = marker.display(),
        ),
    )
    .expect("write plugin");
    fs::write(
        sandbox.project.join("cortex.toml"),
        "plugin = [\"./plugin.mjs\"]\n",
    )
    .expect("write config");
}

struct ProviderFixture {
    handle: Option<std::thread::JoinHandle<()>>,
    base_url: String,
}

impl ProviderFixture {
    fn base_url(&self) -> &str {
        &self.base_url
    }

    fn join(mut self) {
        if let Some(handle) = self.handle.take() {
            handle.join().expect("join provider fixture");
        }
    }
}

fn start_provider_fixture(model_id: &str, expected_key: &str) -> ProviderFixture {
    let listener = bind_loopback_listener();
    let port = listener.local_addr().expect("fixture addr").port();
    listener
        .set_nonblocking(true)
        .expect("set fixture nonblocking");
    let model_id = model_id.to_owned();
    let expected = expected_key.to_owned();
    let handle = std::thread::spawn(move || {
        let started = Instant::now();
        let mut served_authorized = 0usize;
        let mut last_activity = Instant::now();
        while started.elapsed() < Duration::from_secs(8) {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    last_activity = Instant::now();
                    let request = read_http_request(&mut stream);
                    let auth_ok = request
                        .headers
                        .iter()
                        .find_map(|(name, value)| {
                            name.eq_ignore_ascii_case("authorization")
                                .then(|| value == &format!("Bearer {expected}"))
                        })
                        .unwrap_or(false);
                    if !auth_ok {
                        let body = "{\"error\":\"unauthorized\"}";
                        let response = format!(
                            "HTTP/1.1 401 Unauthorized\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{}",
                            body.len(),
                            body
                        );
                        let _ = stream.write_all(response.as_bytes());
                        let _ = stream.flush();
                        continue;
                    }
                    served_authorized += 1;
                    let chunk = serde_json::json!({
                        "id": "auth-fixture",
                        "object": "chat.completion.chunk",
                        "model": model_id,
                        "choices": [{
                            "index": 0,
                            "delta": { "content": "fixture-ok" },
                            "finish_reason": serde_json::Value::Null
                        }]
                    })
                    .to_string();
                    let done = serde_json::json!({
                        "id": "auth-fixture",
                        "object": "chat.completion.chunk",
                        "model": model_id,
                        "choices": [{
                            "index": 0,
                            "delta": {},
                            "finish_reason": "stop"
                        }],
                        "usage": { "prompt_tokens": 3, "completion_tokens": 2, "total_tokens": 5 }
                    })
                    .to_string();
                    let body = format!("data: {chunk}\n\ndata: {done}\n\ndata: [DONE]\n\n");
                    let response = format!(
                        "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncontent-length: {}\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    let _ = stream.write_all(response.as_bytes());
                    let _ = stream.flush();
                }
                Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                    if served_authorized > 0 && last_activity.elapsed() > Duration::from_secs(2) {
                        return;
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(_) => return,
            }
        }
    });
    ProviderFixture {
        handle: Some(handle),
        base_url: format!("http://127.0.0.1:{port}"),
    }
}

fn bind_loopback_listener() -> TcpListener {
    for port in 3200..=3299 {
        if let Ok(listener) = TcpListener::bind(("127.0.0.1", port)) {
            return listener;
        }
    }
    panic!("no free loopback port in 3200-3299");
}

struct ToolCallFixture {
    handle: Option<std::thread::JoinHandle<()>>,
    base_url: String,
}

impl ToolCallFixture {
    fn base_url(&self) -> &str {
        &self.base_url
    }

    fn join(mut self) {
        if let Some(handle) = self.handle.take() {
            handle.join().expect("join tool-call fixture");
        }
    }
}

fn start_tool_call_fixture(tool_name: &str) -> ToolCallFixture {
    let listener = bind_loopback_listener();
    let port = listener.local_addr().expect("fixture addr").port();
    listener
        .set_nonblocking(true)
        .expect("set fixture nonblocking");
    let tool_name = tool_name.to_owned();
    let handle = std::thread::spawn(move || {
        let started = Instant::now();
        let mut sent_tool_call_turn = false;
        let mut sent_final_turn = false;
        while started.elapsed() < Duration::from_secs(8) {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    let request = read_http_request(&mut stream);
                    let request_has_tools = request.body.contains("\"tools\"");
                    let body = if !sent_tool_call_turn && request_has_tools {
                        sent_tool_call_turn = true;
                        format!(
                            "data: {{\"choices\":[{{\"delta\":{{\"tool_calls\":[{{\"index\":0,\"id\":\"call_1\",\"type\":\"function\",\"function\":{{\"name\":\"{tool_name}\",\"arguments\":\"{{}}\"}}}}]}},\"finish_reason\":null}}]}}\n\n\
                             data: {{\"choices\":[{{\"delta\":{{}},\"finish_reason\":\"tool_calls\"}}]}}\n\n\
                             data: [DONE]\n\n"
                        )
                    } else if sent_tool_call_turn {
                        sent_final_turn = true;
                        let tool_result = latest_tool_content(&request.body)
                            .unwrap_or_else(|| "NO_TOOL_RESULT".to_owned());
                        let tool_result_json =
                            serde_json::to_string(&tool_result).expect("tool result json");
                        format!(
                            "data: {{\"choices\":[{{\"delta\":{{\"content\":{tool_result_json}}},\"finish_reason\":null}}]}}\n\n\
                             data: {{\"choices\":[{{\"delta\":{{}},\"finish_reason\":\"stop\"}}],\"usage\":{{\"prompt_tokens\":4,\"completion_tokens\":2,\"total_tokens\":6}}}}\n\n\
                             data: [DONE]\n\n"
                        )
                    } else {
                        "data: {\"choices\":[{\"delta\":{\"content\":\"fixture-warmup\"},\"finish_reason\":null}]}\n\n\
                         data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n\
                         data: [DONE]\n\n"
                            .to_owned()
                    };
                    let response = format!(
                        "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncontent-length: {}\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    let _ = stream.write_all(response.as_bytes());
                    let _ = stream.flush();
                    if sent_final_turn {
                        return;
                    }
                }
                Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(_) => return,
            }
        }
    });

    ToolCallFixture {
        handle: Some(handle),
        base_url: format!("http://127.0.0.1:{port}"),
    }
}

fn latest_tool_content(body: &str) -> Option<String> {
    let parsed: Value = serde_json::from_str(body).ok()?;
    let messages = parsed.get("messages")?.as_array()?;
    messages
        .iter()
        .rev()
        .find(|message| message.get("role").and_then(Value::as_str) == Some("tool"))
        .and_then(|message| message.get("content"))
        .and_then(Value::as_str)
        .map(str::to_owned)
}

struct HttpRequest {
    headers: Vec<(String, String)>,
    body: String,
}

fn read_http_request(stream: &mut TcpStream) -> HttpRequest {
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .expect("set timeout");
    let mut buffer = Vec::new();
    let mut chunk = [0_u8; 1024];
    let header_end = loop {
        let read = stream.read(&mut chunk).expect("read request");
        if read == 0 {
            return HttpRequest {
                headers: Vec::new(),
                body: String::new(),
            };
        }
        buffer.extend_from_slice(&chunk[..read]);
        if let Some(pos) = buffer.windows(4).position(|window| window == b"\r\n\r\n") {
            break pos + 4;
        }
    };
    let header_text = String::from_utf8_lossy(&buffer[..header_end]).to_string();
    let headers = header_text
        .lines()
        .skip(1)
        .filter_map(|line| {
            let (name, value) = line.split_once(':')?;
            Some((name.trim().to_owned(), value.trim().to_owned()))
        })
        .collect::<Vec<_>>();
    let content_length = headers
        .iter()
        .find_map(|(name, value)| {
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.parse::<usize>().ok())
                .flatten()
        })
        .unwrap_or(0);
    while buffer.len() < header_end + content_length {
        let read = stream.read(&mut chunk).expect("read request body");
        if read == 0 {
            break;
        }
        buffer.extend_from_slice(&chunk[..read]);
    }
    let body_end = std::cmp::min(buffer.len(), header_end + content_length);
    let body = String::from_utf8_lossy(&buffer[header_end..body_end]).to_string();
    HttpRequest { headers, body }
}

fn fake_openrouter_key() -> String {
    format!("sk-{}-{}", "or-v1", "abcdef01".repeat(8))
}
