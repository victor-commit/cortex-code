use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
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

#[test]
fn plugin_auth_models_and_provider_surfaces_are_discoverable() {
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

    let auth = run(
        cortex(&sandbox, &["auth", "list", "--format", "json"]),
        Duration::from_secs(30),
    );
    assert_eq!(auth.status.code(), Some(0));
    let auth_json: serde_json::Value =
        serde_json::from_slice(&auth.stdout).expect("auth json parses");
    assert!(
        auth_json["methods"]
            .as_array()
            .unwrap()
            .iter()
            .any(|method| method["id"] == "plugin-auth" && method["source"] == "plugin"),
        "plugin auth method should be listed: {auth_json}"
    );

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

    let auth = run(
        cortex(&sandbox, &["auth", "list", "--format", "json"]),
        Duration::from_secs(10),
    );
    assert_eq!(auth.status.code(), Some(0));
    let auth_json: serde_json::Value =
        serde_json::from_slice(&auth.stdout).expect("auth json parses");
    assert!(
        !auth_json["methods"]
            .as_array()
            .unwrap()
            .iter()
            .any(|method| method["id"] == "plugin-auth")
    );

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
