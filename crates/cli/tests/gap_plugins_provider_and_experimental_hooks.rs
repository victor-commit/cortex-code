use std::fs;
use std::io::Read;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::Value;

const RUN_TIMEOUT: Duration = Duration::from_secs(40);

struct Sandbox {
    home: PathBuf,
    project: PathBuf,
    data_dir: PathBuf,
}

impl Sandbox {
    fn new(tag: &str) -> Self {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "cortex-gap-plugin-provider-hooks-{tag}-{}-{}",
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

    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_cortex"));
        command.current_dir(&self.project);
        command.env("HOME", &self.home);
        command.env("XDG_CONFIG_HOME", self.home.join(".config"));
        command.env("XDG_DATA_HOME", self.home.join(".local/share"));
        command.env("CORTEX_DATA_DIR", &self.data_dir);
        command
            .env_remove("OPENROUTER_API_KEY")
            .env_remove("CORTEX_AUTH_FILE")
            .env_remove("CORTEX_NO_NET")
            .env_remove("CORTEX_OPENROUTER_BASE_URL")
            .env_remove("CORTEX_MODEL")
            .env_remove("CORTEX_SMALL_MODEL")
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
    let stdout_thread = thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = stdout.read_to_end(&mut bytes);
        bytes
    });
    let stderr_thread = thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = stderr.read_to_end(&mut bytes);
        bytes
    });

    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().expect("try_wait") {
            break status;
        }
        if started.elapsed() > RUN_TIMEOUT {
            let _ = child.kill();
            let _ = child.wait();
            panic!("command timed out after {RUN_TIMEOUT:?}");
        }
        thread::sleep(Duration::from_millis(20));
    };

    Output {
        status,
        stdout: stdout_thread.join().expect("join stdout"),
        stderr: stderr_thread.join().expect("join stderr"),
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

fn skip_without_runtime() -> bool {
    if plugins::detect_runtime().is_none() {
        eprintln!("skipping plugin provider gap test: no Bun, Node, or Deno on PATH");
        true
    } else {
        false
    }
}

#[test]
fn provider_hook_streams_with_auth_and_reports_model_metadata_and_cost() {
    if skip_without_runtime() {
        return;
    }

    let sandbox = Sandbox::new("stream-auth-meta");
    let marker = sandbox.project.join("provider-auth.jsonl");
    let plugin = sandbox.project.join("plugin.mjs");
    fs::write(
        &plugin,
        r#"
import { appendFile } from "node:fs/promises";

export default async function plugin(input, options) {
  return {
    auth: {
      provider: "plugin-provider",
      name: "Plugin Provider Auth",
      loader: async (auth) => {
        const value = await auth();
        return {
          token: String(value?.key ?? value?.token ?? "")
        };
      },
      methods: [{
        id: "api",
        type: "api",
        label: "API key",
        prompts: [{ type: "text", key: "api_key", message: "API key" }],
        async authorize(inputs) {
          return {
            type: "success",
            credential: { type: "api", key: String(inputs?.api_key ?? "") }
          };
        }
      }]
    },
    async provider() {
      return {
        id: "plugin-provider",
        name: "Plugin Provider",
        models: [{
          id: "plugin/stream-model",
          name: "Plugin Stream Model",
          tool_call: false,
          attachment: true,
          cost: { input: 2, output: 4 },
          context: 8192,
          output: 1024
        }],
        async complete(request) {
          await appendFile(options.marker, JSON.stringify({
            model: request.model,
            auth: request.auth ?? null
          }) + "\n");
          return {
            events: [
              { type: "text_delta", text: "stream-" },
              { type: "text_delta", text: "ok" },
              { type: "usage", input_tokens: 1000, output_tokens: 2000, total_tokens: 3000 },
              { type: "done", reason: "stop" }
            ]
          };
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
            r#"
plugin = [[ "{}", {{ marker = "{}" }} ]]
"#,
            plugin.display(),
            marker.display()
        ),
    )
    .expect("write config");

    let mut login = sandbox.command();
    login.args([
        "auth",
        "login",
        "plugin-provider",
        "--auth-method",
        "api",
        "--auth-input",
        "api_key=tok-123",
        "--format",
        "json",
    ]);
    assert_success("auth login", &run(login));

    let mut models = sandbox.command();
    models.args(["models", "--format", "json"]);
    let models_out = run(models);
    assert_success("models", &models_out);
    let models_json: Value = serde_json::from_slice(&models_out.stdout).expect("models json");
    let plugin_model = models_json["providers"]
        .as_array()
        .expect("providers array")
        .iter()
        .find(|provider| provider["id"] == "plugin-provider")
        .and_then(|provider| provider["models"].as_array())
        .and_then(|models| {
            models
                .iter()
                .find(|model| model["id"] == "plugin/stream-model")
        })
        .expect("plugin model in models list");
    assert_eq!(plugin_model["attachment"], true, "attachment metadata");
    assert_eq!(plugin_model["cost"]["input"], 2.0, "input price metadata");
    assert_eq!(plugin_model["cost"]["output"], 4.0, "output price metadata");

    let mut run_cmd = sandbox.command();
    run_cmd.args([
        "run",
        "--jsonl",
        "--model",
        "plugin/stream-model",
        "stream with plugin provider",
    ]);
    let run_out = run(run_cmd);
    assert_success("run --jsonl", &run_out);
    let events = String::from_utf8_lossy(&run_out.stdout)
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str::<Value>(line).expect("jsonl line"))
        .collect::<Vec<_>>();
    let text_deltas = events
        .iter()
        .filter(|event| event["type"] == "text_delta")
        .count();
    assert!(
        text_deltas >= 2,
        "expected streamed deltas, got events: {events:#?}"
    );
    let usage = events
        .iter()
        .find(|event| event["type"] == "usage")
        .expect("usage event");
    let cost = usage["cost"].as_f64().expect("usage cost");
    assert!(
        (cost - 0.01).abs() < f64::EPSILON,
        "plugin usage cost should be computed from plugin pricing metadata: {usage}"
    );
    assert!(
        events.iter().any(|event| event["type"] == "done"),
        "done event should be present: {events:#?}"
    );

    let marker_line = fs::read_to_string(&marker)
        .expect("marker")
        .lines()
        .last()
        .expect("marker line")
        .to_owned();
    let marker_json: Value = serde_json::from_str(&marker_line).expect("marker json");
    assert_eq!(marker_json["auth"]["token"], "tok-123");
}

#[test]
fn small_model_and_compaction_experimental_hooks_transform_real_compaction() {
    if skip_without_runtime() {
        return;
    }

    let sandbox = Sandbox::new("small-model-compaction-hooks");
    let marker = sandbox.project.join("hooks.jsonl");
    let plugin = sandbox.project.join("plugin.mjs");
    fs::write(
        &plugin,
        r#"
import { appendFile } from "node:fs/promises";

export default async function plugin(input, options) {
  async function mark(payload) {
    await appendFile(options.marker, JSON.stringify(payload) + "\n");
  }

  return {
    experimental: {
      provider: {
        async small_model(current) {
          await mark({ hook: "experimental.provider.small_model", current });
          return { model: "plugin/small-model" };
        }
      },
      session: {
        async compacting(record) {
          await mark({ hook: "experimental.session.compacting", model: record.model ?? null });
          return {
            ...record,
            summary: `HOOKED:${record.summary}`
          };
        }
      },
      compaction: {
        async autocontinue(record) {
          await mark({ hook: "experimental.compaction.autocontinue", model: record.model ?? null });
          return {
            ...record,
            continued_request: false
          };
        }
      }
    },
    async provider() {
      return {
        id: "plugin-provider",
        name: "Plugin Provider",
        models: [
          { id: "plugin/main-model", name: "Main Model", context: 4096, output: 512 },
          { id: "plugin/small-model", name: "Small Model", context: 4096, output: 512 }
        ],
        async complete(request) {
          await mark({ hook: "provider.complete", model: request.model });
          if (request.model === "plugin/small-model") {
            return { content: "small-summary" };
          }
          return { content: "main-answer" };
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
            r#"
plugin = [[ "{}", {{ marker = "{}" }} ]]
"#,
            plugin.display(),
            marker.display()
        ),
    )
    .expect("write config");

    let session = "gap-small-model-session";
    let filler = "padding context line ".repeat(600);
    for i in 0..8 {
        let prompt = format!("{filler}\nTurn {i}: acknowledge with one token.");
        let mut seed_cmd = sandbox.command();
        seed_cmd.args(["run", "--session", session, "--model", "plugin/main-model"]);
        seed_cmd.env("CORTEX_CONTEXT_LIMIT_TOKENS", "2048");
        seed_cmd.arg(&prompt);
        let seed_out = run(seed_cmd);
        assert_success("seed run", &seed_out);
    }

    let final_prompt = format!("{filler}\nReply with exactly: main-answer");
    let mut run_cmd = sandbox.command();
    run_cmd.args([
        "run",
        "--format",
        "json",
        "--session",
        session,
        "--model",
        "plugin/main-model",
    ]);
    run_cmd.env("CORTEX_CONTEXT_LIMIT_TOKENS", "2048");
    run_cmd.arg(&final_prompt);
    let run_out = run(run_cmd);
    assert_success("run --format json", &run_out);
    let run_json: Value = serde_json::from_slice(&run_out.stdout).expect("run json");
    assert_eq!(run_json["content"], "main-answer");
    let compactions = run_json["compactions"]
        .as_array()
        .expect("compactions array");
    assert!(
        !compactions.is_empty(),
        "expected at least one compaction record: {run_json}"
    );
    let first = &compactions[0];
    assert!(
        first["summary"]
            .as_str()
            .is_some_and(|summary| summary.starts_with("HOOKED:")),
        "session.compacting should transform summary: {first}"
    );
    assert_eq!(
        first["continued_request"], false,
        "autocontinue hook should transform continuation flag"
    );
    assert_eq!(
        first["model"], "plugin/small-model",
        "small-model hook selection should be reported"
    );

    let entries = fs::read_to_string(&marker)
        .expect("marker")
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str::<Value>(line).expect("marker line"))
        .collect::<Vec<_>>();
    let index = |name: &str| {
        entries
            .iter()
            .position(|entry| entry["hook"] == name)
            .unwrap_or_else(|| panic!("missing marker hook `{name}`: {entries:#?}"))
    };
    let i_small = index("experimental.provider.small_model");
    let i_compacting = index("experimental.session.compacting");
    let i_autocontinue = index("experimental.compaction.autocontinue");
    assert!(
        i_small < i_compacting && i_compacting < i_autocontinue,
        "hook order should be small_model -> session.compacting -> compaction.autocontinue: {entries:#?}"
    );
    assert!(
        entries.iter().any(|entry| {
            entry["hook"] == "provider.complete" && entry["model"] == "plugin/small-model"
        }),
        "provider should run compaction summary through selected small model: {entries:#?}"
    );
}
