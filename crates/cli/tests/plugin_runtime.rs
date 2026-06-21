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
