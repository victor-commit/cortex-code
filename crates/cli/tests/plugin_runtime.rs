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

fn executable_on_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|directory| directory.join(name))
        .find(|candidate| candidate.is_file())
}

#[cfg(target_os = "linux")]
fn assert_runtime_reaped(pid: u64) {
    let process_path = PathBuf::from(format!("/proc/{pid}"));
    let deadline = Instant::now() + Duration::from_secs(2);
    while process_path.exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        !process_path.exists(),
        "plugin runtime child PID {pid} must be reaped after cortex exits"
    );
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

#[cfg(target_os = "linux")]
#[test]
fn cortex_run_uses_bun_then_node_deterministically_and_reaps_each_runtime() {
    let Some(bun) = executable_on_path("bun") else {
        eprintln!("skipping runtime precedence proof: Bun is unavailable");
        return;
    };
    let Some(node) = executable_on_path("node") else {
        eprintln!("skipping runtime precedence proof: Node is unavailable");
        return;
    };

    let sandbox = Sandbox::new("runtime-precedence");
    let runtime_bin = sandbox.home.join("runtime-bin");
    std::fs::create_dir_all(&runtime_bin).expect("create controlled runtime path");
    std::os::unix::fs::symlink(&bun, runtime_bin.join("bun")).expect("link Bun");
    std::os::unix::fs::symlink(&node, runtime_bin.join("node")).expect("link Node");

    let marker = sandbox.project.join("runtime.json");
    write_plugin(
        &sandbox.project.join("plugin.mjs"),
        &format!(
            r#"
import {{ writeFile }} from "node:fs/promises";

export default async function plugin() {{
  return {{
    async event() {{
      const runtime = globalThis.Bun ? "bun" : "node";
      await writeFile("{}", JSON.stringify({{ runtime, pid: process.pid }}));
    }}
  }};
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

    for expected_runtime in ["bun", "bun"] {
        let mut command = cortex(
            &sandbox,
            &["run", "--command", "echo", "--command-arg", "runtime proof"],
        );
        command.env("PATH", &runtime_bin);
        let output = run(command, Duration::from_secs(30));
        assert_eq!(
            output.status.code(),
            Some(0),
            "cortex run should succeed: stderr={}",
            String::from_utf8_lossy(&output.stderr)
        );

        let payload: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&marker).expect("runtime marker"))
                .expect("runtime marker is JSON");
        assert_eq!(
            payload["runtime"], expected_runtime,
            "Bun must win deterministically while both runtimes are present"
        );
        assert_runtime_reaped(payload["pid"].as_u64().expect("runtime PID"));
    }

    std::fs::remove_file(runtime_bin.join("bun")).expect("remove top-precedence runtime");
    std::fs::remove_file(&marker).expect("clear Bun marker");

    let mut command = cortex(
        &sandbox,
        &[
            "run",
            "--command",
            "echo",
            "--command-arg",
            "Node fallback proof",
        ],
    );
    command.env("PATH", &runtime_bin);
    let output = run(command, Duration::from_secs(30));
    assert_eq!(
        output.status.code(),
        Some(0),
        "cortex run should fall through to Node: stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );

    let payload: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&marker).expect("Node marker"))
            .expect("Node marker is JSON");
    assert_eq!(
        payload["runtime"], "node",
        "removing Bun must deterministically select Node"
    );
    assert_runtime_reaped(payload["pid"].as_u64().expect("runtime PID"));
}

#[test]
fn cortex_run_stops_loading_a_removed_plugin_config_entry() {
    if skip_without_runtime() {
        return;
    }

    let sandbox = Sandbox::new("config-removal");
    let marker = sandbox.project.join("plugin.marker");
    write_plugin(
        &sandbox.project.join("plugin.mjs"),
        &format!(
            r#"
import {{ writeFile }} from "node:fs/promises";

export default async function plugin() {{
  return {{
    async event() {{
      await writeFile("{}", "plugin-only effect");
    }}
  }};
}}
"#,
            marker.display()
        ),
    );
    let config = sandbox.project.join("cortex.toml");
    std::fs::write(&config, "plugin = [\"./plugin.mjs\"]\n").expect("write config");

    let loaded = run(
        cortex(
            &sandbox,
            &[
                "run",
                "--command",
                "echo",
                "--command-arg",
                "plugin configured",
            ],
        ),
        Duration::from_secs(30),
    );
    assert_eq!(loaded.status.code(), Some(0));
    assert!(
        marker.exists(),
        "configured plugin must cause its own effect"
    );

    std::fs::remove_file(&marker).expect("remove plugin marker");
    std::fs::remove_file(config).expect("remove plugin config");
    let empty_path = sandbox.home.join("empty-path");
    std::fs::create_dir_all(&empty_path).expect("create empty runtime path");
    let mut without_plugin = cortex(
        &sandbox,
        &[
            "run",
            "--command",
            "echo",
            "--command-arg",
            "plugin removed",
        ],
    );
    without_plugin.env("PATH", empty_path);
    let removed = run(without_plugin, Duration::from_secs(10));
    assert_eq!(
        removed.status.code(),
        Some(0),
        "no configured plugin must not require a JavaScript runtime: stderr={}",
        String::from_utf8_lossy(&removed.stderr)
    );
    assert!(
        !marker.exists(),
        "removing the config entry must remove the plugin-only effect"
    );
}

#[test]
fn cortex_run_config_pair_options_change_a_local_plugin_effect() {
    if skip_without_runtime() {
        return;
    }

    let sandbox = Sandbox::new("local-options");
    let marker = sandbox.project.join("option-marker.txt");
    write_plugin(
        &sandbox.project.join("plugin.mjs"),
        r#"
import { writeFile } from "node:fs/promises";

export default async function plugin(_input, options) {
  return {
    async event() {
      await writeFile(options.marker, `label=${options.label}`);
    }
  };
}
"#,
    );
    let config = sandbox.project.join("cortex.toml");
    std::fs::write(
        &config,
        format!(
            "plugin = [[\"./plugin.mjs\", {{ marker = \"{}\", label = \"L1\" }}]]\n",
            marker.display()
        ),
    )
    .expect("write L1 config");

    let l1 = run(
        cortex(
            &sandbox,
            &["run", "--command", "echo", "--command-arg", "options"],
        ),
        Duration::from_secs(30),
    );
    assert_eq!(
        l1.status.code(),
        Some(0),
        "L1 run succeeds: {}",
        String::from_utf8_lossy(&l1.stderr)
    );
    assert_eq!(std::fs::read_to_string(&marker).unwrap(), "label=L1");

    std::fs::write(
        &config,
        format!(
            "plugin = [[\"./plugin.mjs\", {{ marker = \"{}\", label = \"L2\" }}]]\n",
            marker.display()
        ),
    )
    .expect("write L2 config");
    let l2 = run(
        cortex(
            &sandbox,
            &["run", "--command", "echo", "--command-arg", "options"],
        ),
        Duration::from_secs(30),
    );
    assert_eq!(
        l2.status.code(),
        Some(0),
        "L2 run succeeds: {}",
        String::from_utf8_lossy(&l2.stderr)
    );
    assert_eq!(std::fs::read_to_string(&marker).unwrap(), "label=L2");
}

#[test]
fn cortex_run_reports_missing_and_syntax_invalid_local_plugin_entries() {
    if skip_without_runtime() {
        return;
    }

    let missing = Sandbox::new("missing-plugin");
    std::fs::write(
        missing.project.join("cortex.toml"),
        "plugin = [\"./does-not-exist.mjs\"]\n",
    )
    .expect("write missing plugin config");
    let missing_output = run(
        cortex(
            &missing,
            &["run", "--command", "echo", "--command-arg", "missing"],
        ),
        Duration::from_secs(30),
    );
    assert_ne!(missing_output.status.code(), Some(0));
    let missing_stderr = String::from_utf8_lossy(&missing_output.stderr);
    assert!(
        missing_stderr.contains("does-not-exist.mjs")
            && missing_stderr.contains("does not exist")
            && !missing_stderr.contains("panicked"),
        "missing entry fails with an attributed, non-panicking diagnostic: {missing_stderr}"
    );

    let syntax = Sandbox::new("syntax-plugin");
    let plugin = syntax.project.join("syntax-error.mjs");
    write_plugin(
        &plugin,
        r#"
export default async function plugin() {
  return {
    event: async () => {
      this is not valid JavaScript;
    }
  };
}
"#,
    );
    std::fs::write(
        syntax.project.join("cortex.toml"),
        "plugin = [\"./syntax-error.mjs\"]\n",
    )
    .expect("write syntax plugin config");
    let syntax_output = run(
        cortex(
            &syntax,
            &["run", "--command", "echo", "--command-arg", "syntax"],
        ),
        Duration::from_secs(30),
    );
    assert_ne!(syntax_output.status.code(), Some(0));
    let syntax_stderr = String::from_utf8_lossy(&syntax_output.stderr);
    assert!(
        syntax_stderr.contains("syntax-error.mjs")
            && syntax_stderr.to_ascii_lowercase().contains("syntax")
            && !syntax_stderr.contains("panicked"),
        "syntax error names the offending entry without a panic: {syntax_stderr}"
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
fn cortex_pure_env_skips_plugin_hooks_and_registered_models() {
    if skip_without_runtime() {
        return;
    }

    let sandbox = Sandbox::new("pure-env");
    let marker = sandbox.project.join("pure-env.marker");
    write_plugin(
        &sandbox.project.join("plugin.mjs"),
        &format!(
            r#"
import {{ writeFile }} from "node:fs/promises";

export default async function plugin() {{
  await writeFile("{}", "plugin-loaded");
  return {{
    async auth() {{
      return {{ id: "plugin-auth", name: "Plugin Auth" }};
    }},
    async provider() {{
      return {{
        id: "plugin-provider",
        name: "Plugin Provider",
        models: [{{ id: "plugin/model", name: "Plugin Model" }}]
      }};
    }}
  }};
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

    let loaded_models = run(
        cortex(&sandbox, &["models", "--format", "json"]),
        Duration::from_secs(30),
    );
    assert_eq!(loaded_models.status.code(), Some(0));
    assert!(
        String::from_utf8_lossy(&loaded_models.stdout).contains("plugin/model"),
        "normal mode materializes the plugin provider"
    );
    assert!(marker.exists(), "normal mode executed the plugin");

    std::fs::remove_file(&marker).expect("clear normal plugin marker");
    let mut pure_models = cortex(&sandbox, &["models", "--pure", "--format", "json"]);
    pure_models.env("CORTEX_PURE", "1");
    let pure_models = run(pure_models, Duration::from_secs(10));
    assert_eq!(pure_models.status.code(), Some(0));
    assert!(
        !String::from_utf8_lossy(&pure_models.stdout).contains("plugin/model"),
        "--pure removes plugin models/providers"
    );
    assert!(
        !marker.exists(),
        "CORTEX_PURE=1 must suppress every plugin load side effect"
    );
}

/// Whether `strace` can trace a child here (it may be absent or `ptrace` may be restricted).
fn strace_usable() -> bool {
    let probe = Command::new("strace")
        .args([
            "-f",
            "-e",
            "trace=connect",
            env!("CARGO_BIN_EXE_cortex"),
            "--version",
        ])
        .output();
    match probe {
        Ok(out) => {
            let stderr = String::from_utf8_lossy(&out.stderr);
            out.status.success() && !stderr.to_lowercase().contains("ptrace")
        }
        Err(_) => false,
    }
}

fn count_connect(trace: &str) -> usize {
    trace.matches("connect(").count()
}

/// `CORTEX_NO_NET=1` must stop plugin session startup from performing loopback TCP
/// discovery (or any other connect) on the `cortex run --command` branch.
///
/// Without the kill-switch, `RuntimeHost`/`PluginRuntimeContext` scans ports
/// 3200-3299 via real `connect()` health checks even when no serve instance is
/// listening. That must not happen under the network kill-switch.
#[test]
fn cortex_run_command_honors_no_net_before_plugin_loopback_discovery() {
    if skip_without_runtime() {
        return;
    }
    if !strace_usable() {
        eprintln!("skipping strace assertion: strace unavailable or ptrace restricted");
        return;
    }

    let sandbox = Sandbox::new("command-no-net");
    let marker = sandbox.project.join("hook-ran");
    write_plugin(
        &sandbox.project.join("plugin.mjs"),
        &format!(
            r#"
import {{ writeFile }} from "node:fs/promises";

export default async function plugin() {{
  return {{
    "command.execute.before": async () => {{
      await writeFile("{}", "command-hook");
    }}
  }};
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

    // Positive control: without the kill-switch the host probes loopback ports and
    // issues real connect() syscalls while loading the configured plugin.
    let mut open = Command::new("strace");
    open.args([
        "-f",
        "-e",
        "trace=connect",
        env!("CARGO_BIN_EXE_cortex"),
        "run",
        "--command",
        "echo",
        "--command-arg",
        "open-net",
    ])
    .current_dir(&sandbox.project)
    .env("HOME", &sandbox.home)
    .env("XDG_CONFIG_HOME", sandbox.home.join(".config"))
    .env("CORTEX_DATA_DIR", sandbox.home.join(".local/share/cortex"))
    .env_remove("CORTEX_NO_NET")
    .env_remove("CORTEX_SERVER_URL")
    .env_remove("OPENROUTER_API_KEY")
    .stdin(Stdio::null())
    .stdout(Stdio::piped())
    .stderr(Stdio::piped());
    let open_out = open.output().expect("spawn strace (network enabled)");
    let open_trace = String::from_utf8_lossy(&open_out.stderr);
    assert!(
        count_connect(&open_trace) >= 1,
        "without CORTEX_NO_NET plugin load must reach real connect() during loopback discovery:\n{open_trace}"
    );

    let _ = std::fs::remove_file(&marker);

    // Kill-switch path: zero connect(), plugin still loads far enough for hooks.
    let mut blocked = Command::new("strace");
    blocked
        .args([
            "-f",
            "-e",
            "trace=connect",
            env!("CARGO_BIN_EXE_cortex"),
            "run",
            "--command",
            "echo",
            "--command-arg",
            "no-net-proof",
        ])
        .current_dir(&sandbox.project)
        .env("HOME", &sandbox.home)
        .env("XDG_CONFIG_HOME", sandbox.home.join(".config"))
        .env("CORTEX_DATA_DIR", sandbox.home.join(".local/share/cortex"))
        .env("CORTEX_NO_NET", "1")
        .env_remove("CORTEX_SERVER_URL")
        .env_remove("OPENROUTER_API_KEY")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let blocked_out = blocked.output().expect("spawn strace (no-net)");
    let blocked_trace = String::from_utf8_lossy(&blocked_out.stderr);
    let blocked_stdout = String::from_utf8_lossy(&blocked_out.stdout);
    // strace merges cortex stderr with its own connect trace; the process exits via
    // cortex, so status is on the outer strace process (same as cortex's exit).
    assert_eq!(
        blocked_out.status.code(),
        Some(0),
        "local --command with a real plugin must still succeed under CORTEX_NO_NET: stdout={blocked_stdout} stderr={blocked_trace}"
    );
    assert_eq!(
        blocked_stdout.trim(),
        "no-net-proof",
        "command echo must still run under the kill-switch"
    );
    assert_eq!(
        count_connect(&blocked_trace),
        0,
        "under CORTEX_NO_NET=1 plugin load must issue zero connect() syscalls:\n{blocked_trace}"
    );
    assert_eq!(
        std::fs::read_to_string(&marker).expect("hook marker written"),
        "command-hook",
        "local plugin must still load and run command.execute.before under NO_NET (discovery skipped, hooks still active)"
    );
}

/// `--pure` / `CORTEX_PURE=1` must short-circuit plugin-management validation probes so install
/// and update neither resolve package plugins nor execute plugin load side effects (VAL-PLUGIN-032).
#[test]
fn cortex_pure_plugin_install_and_update_neither_resolve_nor_execute() {
    if skip_without_runtime() {
        return;
    }

    let sandbox = Sandbox::new("pure-mgmt");
    let package = sandbox.project.join("pkg");
    let marker = sandbox.home.join("markers/pure-mgmt-marker.txt");
    std::fs::create_dir_all(marker.parent().expect("marker parent")).expect("marker dir");
    write_package_plugin(
        &package,
        "pure-mgmt-proof",
        "1.2.3",
        &format!(
            r#"
import {{ appendFile }} from "node:fs/promises";
export default async function plugin() {{
  await appendFile("{}", "loaded\n");
  return {{ async event() {{}} }};
}}
"#,
            marker.display()
        ),
    );
    let spec = format!("file:{}", package.display());

    // Positive control: ordinary install resolves, loads, and leaves a load-side-effect marker.
    let install = run(
        cortex(&sandbox, &["plugin", "install", &spec, "--format", "json"]),
        Duration::from_secs(30),
    );
    assert_eq!(
        install.status.code(),
        Some(0),
        "non-pure install should succeed: {}",
        String::from_utf8_lossy(&install.stderr)
    );
    let install_doc: serde_json::Value =
        serde_json::from_slice(&install.stdout).expect("install JSON");
    assert_eq!(install_doc["pure"], false);
    assert!(
        marker.exists(),
        "non-pure install validation probe must load plugin code"
    );
    let marker_after_install =
        std::fs::read_to_string(&marker).expect("marker after non-pure install");
    assert!(
        !marker_after_install.is_empty(),
        "non-pure install must write a load marker"
    );
    let lock_path = sandbox.project.join(".cortex/plugin-lock.json");
    let lock_after_install = std::fs::read_to_string(&lock_path).expect("lockfile after install");
    assert!(
        lock_after_install.contains("pure-mgmt-proof") && lock_after_install.contains("1.2.3"),
        "non-pure install pins the package: {lock_after_install}"
    );

    // Pure install: no resolve/load of plugin code (marker unchanged; lock not re-resolved).
    std::fs::remove_file(&marker).expect("clear marker before pure install");
    let pure_install = run(
        cortex(
            &sandbox,
            &["--pure", "plugin", "install", &spec, "--format", "json"],
        ),
        Duration::from_secs(15),
    );
    assert_eq!(
        pure_install.status.code(),
        Some(0),
        "pure plugin install should skip probe cleanly: {}",
        String::from_utf8_lossy(&pure_install.stderr)
    );
    let pure_install_doc: serde_json::Value =
        serde_json::from_slice(&pure_install.stdout).expect("pure install JSON");
    assert_eq!(pure_install_doc["pure"], true);
    assert!(
        !marker.exists(),
        "pure plugin install must not execute configured plugin code"
    );
    // Reinstall under pure should not rewrite a fresh lock pin from a resolve probe.
    let lock_after_pure_install =
        std::fs::read_to_string(&lock_path).expect("lockfile after pure install");
    assert_eq!(
        lock_after_install, lock_after_pure_install,
        "pure install must not re-resolve or repin lock entries"
    );

    // Pure update via environment equivalent: leave the lock pin alone and never execute plugin code.
    std::fs::write(&marker, "pre-pure-update\n").expect("seed marker");
    let mut pure_update = cortex(&sandbox, &["plugin", "update", &spec, "--format", "json"]);
    pure_update.env("CORTEX_PURE", "1");
    let pure_update = run(pure_update, Duration::from_secs(15));
    assert_eq!(
        pure_update.status.code(),
        Some(0),
        "pure plugin update should skip re-resolve cleanly: {}",
        String::from_utf8_lossy(&pure_update.stderr)
    );
    let pure_update_doc: serde_json::Value =
        serde_json::from_slice(&pure_update.stdout).expect("pure update JSON");
    assert_eq!(pure_update_doc["pure"], true);
    assert!(pure_update_doc.get("locked").is_none() || pure_update_doc["locked"].is_null());
    let marker_after_pure_update =
        std::fs::read_to_string(&marker).expect("marker after pure update");
    assert_eq!(
        marker_after_pure_update, "pre-pure-update\n",
        "pure plugin update must not resolve or execute plugin code"
    );
    let lock_after_pure_update =
        std::fs::read_to_string(&lock_path).expect("lockfile after pure update");
    assert_eq!(
        lock_after_install, lock_after_pure_update,
        "pure update must not drop or re-resolve the lock pin"
    );

    // Ordinary (non-pure) update continues to re-resolve and execute the load probe.
    std::fs::remove_file(&marker).expect("clear marker before non-pure update");
    let update = run(
        cortex(&sandbox, &["plugin", "update", &spec, "--format", "json"]),
        Duration::from_secs(30),
    );
    assert_eq!(
        update.status.code(),
        Some(0),
        "non-pure plugin update should re-resolve: {}",
        String::from_utf8_lossy(&update.stderr)
    );
    let update_doc: serde_json::Value =
        serde_json::from_slice(&update.stdout).expect("update JSON");
    assert_eq!(update_doc["pure"], false);
    assert!(
        marker.exists(),
        "non-pure update validation probe must load plugin code"
    );
}

/// Empty plugin specs and pure-mode inventory routing for `plugin install` remain fail-closed.
#[test]
fn cortex_pure_plugin_install_still_rejects_empty_spec() {
    let sandbox = Sandbox::new("pure-empty-spec");
    let pure_empty = run(
        cortex(&sandbox, &["--pure", "plugin", "install", ""]),
        Duration::from_secs(10),
    );
    assert_eq!(
        pure_empty.status.code(),
        Some(2),
        "empty plugin specs remain a usage error under pure mode: {}",
        String::from_utf8_lossy(&pure_empty.stderr)
    );
    let err = String::from_utf8_lossy(&pure_empty.stderr);
    assert!(
        err.contains("must not be empty"),
        "pure install should still validate the management CLI surface: {err}"
    );
}
