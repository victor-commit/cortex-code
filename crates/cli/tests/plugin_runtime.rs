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
            "cortex-cli-plugin-{tag}-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let project = home.join("project");
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(&project).expect("create project");
        Self { home, project }
    }
}

fn write_plugin(path: &Path, body: &str) {
    std::fs::write(path, body).expect("write plugin");
}

fn write_package_plugin(root: &Path, name: &str, version: &str, body: &str) {
    std::fs::create_dir_all(root).expect("create package root");
    std::fs::write(
        root.join("package.json"),
        format!(
            r#"{{
  "name": "{name}",
  "version": "{version}",
  "type": "module",
  "main": "index.mjs"
}}
"#
        ),
    )
    .expect("write package json");
    std::fs::write(root.join("index.mjs"), body).expect("write package entry");
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
            panic!("cortex run did not terminate within {timeout:?}");
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
        eprintln!("skipping plugin CLI proof: no Bun, Node, or Deno on PATH");
        true
    } else {
        false
    }
}

#[test]
fn cortex_run_loads_configured_local_plugin_before_model_setup() {
    if skip_without_runtime() {
        return;
    }

    let sandbox = Sandbox::new("load");
    let marker = sandbox.project.join("loaded.json");
    write_plugin(
        &sandbox.project.join("plugin.mjs"),
        r#"
import { writeFile } from "node:fs/promises";

export default async function plugin(input, options) {
  return {
    async event() {
      const runtime = globalThis.Bun
        ? `bun:${Bun.version}`
        : globalThis.Deno
          ? `deno:${Deno.version.deno}`
          : `node:${process.versions.node}`;
      const shell = await input.shell("printf cli-shell-ok");
      await writeFile(options.marker, JSON.stringify({
        directory: input.directory,
        handles: Boolean(input.client && input.server),
        shell: shell.stdout,
        runtime,
        pid: process.pid
      }));
    }
  };
}
"#,
    );
    std::fs::write(
        sandbox.project.join("cortex.toml"),
        format!(
            "plugin = [[\"./plugin.mjs\", {{ marker = \"{}\" }}]]\n",
            marker.display()
        ),
    )
    .expect("write config");

    let output = run(
        cortex(&sandbox, &["run", "plugin load proof"]),
        Duration::from_secs(30),
    );

    assert_ne!(
        output.status.code(),
        Some(0),
        "the keyless run should stop after plugin load at credential setup"
    );
    let marker_json: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&marker).expect("plugin marker exists"))
            .expect("marker json");
    assert_eq!(
        marker_json["directory"],
        sandbox.project.display().to_string()
    );
    assert_eq!(marker_json["handles"], true);
    assert_eq!(marker_json["shell"], "cli-shell-ok");
    assert!(
        marker_json["runtime"]
            .as_str()
            .is_some_and(|runtime| runtime.starts_with("bun:")
                || runtime.starts_with("node:")
                || runtime.starts_with("deno:")),
        "runtime marker proves real JS execution: {marker_json}"
    );
    assert!(
        marker_json["pid"].as_u64().is_some_and(|pid| pid > 0),
        "runtime pid recorded: {marker_json}"
    );
}

#[test]
fn cortex_run_fires_plugin_event_hook_for_turn_lifecycle() {
    if skip_without_runtime() {
        return;
    }

    let sandbox = Sandbox::new("run-turn-event");
    let marker = sandbox.project.join("turn-events.log");
    write_plugin(
        &sandbox.project.join("plugin.mjs"),
        r#"
import { appendFile } from "node:fs/promises";

export default async function plugin(input, options) {
  return {
    async event(event) {
      if (event.kind === "turn.started") {
        await appendFile(options.marker, `${event.surface}:${event.kind}\n`);
      }
    },
    async provider() {
      return {
        id: "plugin-lifecycle",
        name: "Plugin Lifecycle",
        models: [{ id: "plugin/lifecycle-model", name: "Lifecycle Model", tool_call: false }],
        async complete(request) {
          return `PLUGIN_LIFECYCLE:${request.model}`;
        }
      };
    }
  };
}
"#,
    );
    std::fs::write(
        sandbox.project.join("cortex.toml"),
        format!(
            "plugin = [[\"./plugin.mjs\", {{ marker = \"{}\" }}]]\n",
            marker.display()
        ),
    )
    .expect("write config");

    let output = run(
        cortex(
            &sandbox,
            &[
                "run",
                "--model",
                "plugin/lifecycle-model",
                "--format",
                "json",
                "trigger the plugin lifecycle event",
            ],
        ),
        Duration::from_secs(30),
    );

    assert_eq!(output.status.code(), Some(0));
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).expect("run json parses");
    assert!(
        json["content"]
            .as_str()
            .unwrap_or_default()
            .contains("PLUGIN_LIFECYCLE:plugin/lifecycle-model"),
        "plugin provider should produce the answer: {json}"
    );
    let marker_text = std::fs::read_to_string(&marker).unwrap_or_default();
    assert!(
        marker_text.lines().any(|line| line == "run:turn.started"),
        "headless turns should invoke plugin event hooks with run surface: {marker_text:?}"
    );
}

#[test]
fn cortex_run_fails_closed_when_configured_plugin_has_no_runtime() {
    let sandbox = Sandbox::new("no-runtime");
    let empty_path = sandbox.home.join("empty-path");
    std::fs::create_dir_all(&empty_path).expect("empty path");
    let marker = sandbox.project.join("absent.marker");
    write_plugin(
        &sandbox.project.join("plugin.mjs"),
        &format!(
            r#"
import {{ writeFile }} from "node:fs/promises";
export default async function plugin() {{
  await writeFile("{}", "should-not-run");
}}
"#,
            marker.display()
        ),
    );
    std::fs::write(
        sandbox.project.join("cortex.toml"),
        "plugin = [\"./plugin.mjs\"]\n",
    )
    .expect("write config");

    let mut command = cortex(&sandbox, &["run", "no runtime proof"]);
    command.env("PATH", &empty_path);
    let output = run(command, Duration::from_secs(10));

    assert_ne!(output.status.code(), Some(0));
    assert!(
        !marker.exists(),
        "plugin side effect must be absent when runtime detection fails"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("no JavaScript runtime") && stderr.contains("Bun, Node, or Deno"),
        "stderr gives a clear runtime requirement: {stderr}"
    );
}

#[test]
fn cortex_run_attributes_plugin_load_throw_to_the_plugin_path() {
    if skip_without_runtime() {
        return;
    }

    let sandbox = Sandbox::new("throw");
    let marker = sandbox.project.join("should-not-exist");
    let plugin = sandbox.project.join("thrower.mjs");
    write_plugin(
        &plugin,
        r#"
throw new Error("boom-cli-plugin");
export default function plugin() {}
"#,
    );
    std::fs::write(
        sandbox.project.join("cortex.toml"),
        "plugin = [\"./thrower.mjs\"]\n",
    )
    .expect("write config");

    let output = run(
        cortex(&sandbox, &["run", "throw proof"]),
        Duration::from_secs(30),
    );

    assert_ne!(output.status.code(), Some(0));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("thrower.mjs") && stderr.contains("boom-cli-plugin"),
        "stderr attributes the throw to the plugin: {stderr}"
    );
    assert!(
        !marker.exists(),
        "a throwing plugin must not leave its would-be hook side effect"
    );
}

#[test]
fn cortex_run_resolves_package_plugin_and_records_lockfile() {
    if skip_without_runtime() {
        return;
    }

    let sandbox = Sandbox::new("package");
    let package = sandbox.project.join("package-plugin");
    let marker = sandbox.project.join("package-marker.txt");
    write_package_plugin(
        &package,
        "cortex-cli-package-plugin-proof",
        "2.3.4",
        r#"
import { writeFile } from "node:fs/promises";

export default async function plugin(input, options) {
  return {
    async event() {
      await writeFile(options.marker, `label=${options.label};dir=${input.directory}`);
    }
  };
}
"#,
    );
    std::fs::write(
        sandbox.project.join("cortex.toml"),
        format!(
            "plugin = [[\"file:{}\", {{ marker = \"{}\", label = \"PKG\" }}]]\n",
            package.display(),
            marker.display()
        ),
    )
    .expect("write config");

    let output = run(
        cortex(
            &sandbox,
            &["run", "--command", "echo", "--command-arg", "ok"],
        ),
        Duration::from_secs(45),
    );

    assert_eq!(
        output.status.code(),
        Some(0),
        "package plugin command path succeeds: stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "ok");
    assert_eq!(
        std::fs::read_to_string(&marker).unwrap(),
        format!("label=PKG;dir={}", sandbox.project.display())
    );
    let lock = std::fs::read_to_string(sandbox.project.join(".cortex/plugin-lock.json"))
        .expect("package lockfile");
    assert!(
        lock.contains("cortex-cli-package-plugin-proof") && lock.contains("2.3.4"),
        "lockfile pins package version: {lock}"
    );

    let mut offline = cortex(
        &sandbox,
        &["run", "--command", "echo", "--command-arg", "again"],
    );
    offline.env("CORTEX_NO_NET", "1");
    let output = run(offline, Duration::from_secs(30));
    assert_eq!(
        output.status.code(),
        Some(0),
        "locked package should load without re-resolving/fetching under no-net: stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "again");
}

#[test]
fn cortex_run_package_resolution_honors_no_net_for_missing_package() {
    if skip_without_runtime() {
        return;
    }

    let sandbox = Sandbox::new("package-no-net");
    std::fs::write(
        sandbox.project.join("cortex.toml"),
        "plugin = [\"definitely-missing-cortex-plugin-proof-xyz\"]\n",
    )
    .expect("write config");

    let mut command = cortex(
        &sandbox,
        &["run", "--command", "echo", "--command-arg", "nope"],
    );
    command.env("CORTEX_NO_NET", "1");
    let output = run(command, Duration::from_secs(30));

    assert_ne!(output.status.code(), Some(0));
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("definitely-missing-cortex-plugin-proof-xyz")
            && stderr.contains("CORTEX_NO_NET=1"),
        "missing package fails closed with clear no-net diagnostic: {stderr}"
    );
}

#[test]
fn cortex_run_pure_skips_plugins_hooks_and_runtime_detection() {
    let sandbox = Sandbox::new("pure");
    let empty_path = sandbox.home.join("empty-path");
    std::fs::create_dir_all(&empty_path).expect("empty path");
    let marker = sandbox.project.join("should-not-exist");
    let touched = sandbox.project.join("pure-touched");
    write_plugin(
        &sandbox.project.join("plugin.mjs"),
        &format!(
            r#"
import {{ writeFile }} from "node:fs/promises";
export default async function plugin() {{
  await writeFile("{}", "plugin-ran");
  return {{
    "command.execute.before": async () => {{
      await writeFile("{}", "hook-ran");
    }}
  }};
}}
"#,
            marker.display(),
            marker.display()
        ),
    );
    std::fs::write(
        sandbox.project.join("cortex.toml"),
        "plugin = [\"./plugin.mjs\"]\n",
    )
    .expect("write config");

    let mut command = cortex(&sandbox, &["run", "--pure", "--command", "touch"]);
    command.arg("--command-arg").arg(touched.as_os_str());
    command.env("PATH", &empty_path);
    let output = run(command, Duration::from_secs(10));

    assert_eq!(
        output.status.code(),
        Some(0),
        "--pure should not require a JS runtime: stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        touched.exists(),
        "built-in command path still runs under pure"
    );
    assert!(
        !marker.exists(),
        "pure mode suppresses plugin load and command hooks"
    );
}

#[test]
fn cortex_pure_env_skips_plugin_registered_models() {
    if skip_without_runtime() {
        return;
    }

    let sandbox = Sandbox::new("pure-env");
    write_plugin(
        &sandbox.project.join("plugin.mjs"),
        r#"
export default async function plugin() {
  return {
    async auth() {
      return { id: "plugin-auth", name: "Plugin Auth" };
    },
    async provider() {
      return {
        id: "plugin-provider",
        name: "Plugin Provider",
        models: [{ id: "plugin/model", name: "Plugin Model" }]
      };
    }
  };
}
"#,
    );
    std::fs::write(
        sandbox.project.join("cortex.toml"),
        "plugin = [\"./plugin.mjs\"]\n",
    )
    .expect("write config");

    let pure_models = cortex(&sandbox, &["models", "--pure", "--format", "json"]);
    let pure_models = run(pure_models, Duration::from_secs(10));
    assert_eq!(pure_models.status.code(), Some(0));
    assert!(
        !String::from_utf8_lossy(&pure_models.stdout).contains("plugin/model"),
        "--pure removes plugin models/providers"
    );
}
