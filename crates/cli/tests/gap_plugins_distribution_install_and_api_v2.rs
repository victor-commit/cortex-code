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
            "cortex-cli-plugin-gap-{tag}-{}-{}",
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

fn write_package_plugin(root: &Path, name: &str, version: &str, body: &str, engines: &str) {
    std::fs::create_dir_all(root).expect("create package root");
    std::fs::write(
        root.join("package.json"),
        format!(
            r#"{{
  "name": "{name}",
  "version": "{version}",
  "type": "module",
  "main": "index.mjs",
  "engines": {{ "cortex": "{engines}" }}
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

fn skip_without_runtime() -> bool {
    if plugins::detect_runtime().is_none() {
        eprintln!("skipping plugin gap proof: no Bun, Node, or Deno on PATH");
        true
    } else {
        false
    }
}

fn tuistory_available() -> bool {
    Command::new("tuistory")
        .arg("--version")
        .output()
        .map(|out| out.status.success())
        .unwrap_or(false)
}

struct TuiSessionGuard {
    session: String,
}

impl Drop for TuiSessionGuard {
    fn drop(&mut self) {
        let _ = Command::new("tuistory")
            .arg("-s")
            .arg(&self.session)
            .arg("close")
            .output();
    }
}

fn launch_tui_session(sandbox: &Sandbox, session: &str) -> TuiSessionGuard {
    let command = format!("{} tui", env!("CARGO_BIN_EXE_cortex"));
    let output = Command::new("tuistory")
        .arg("launch")
        .arg("--session")
        .arg(session)
        .arg("--background")
        .arg("--cwd")
        .arg(sandbox.project.display().to_string())
        .arg("--env")
        .arg(format!("HOME={}", sandbox.home.display()))
        .arg("--env")
        .arg(format!(
            "XDG_CONFIG_HOME={}",
            sandbox.home.join(".config").display()
        ))
        .arg("--env")
        .arg(format!(
            "CORTEX_DATA_DIR={}",
            sandbox.home.join(".local/share/cortex").display()
        ))
        .arg("--env")
        .arg("CORTEX_NO_NET=1")
        .arg(command)
        .output()
        .expect("launch tuistory session");
    assert!(
        output.status.success(),
        "tuistory launch failed:\nstdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    TuiSessionGuard {
        session: session.to_owned(),
    }
}

fn tuistory(session: &str, args: &[&str]) -> Output {
    Command::new("tuistory")
        .arg("-s")
        .arg(session)
        .args(args)
        .output()
        .expect("run tuistory command")
}

#[test]
fn gap_plugin_011_tui_type_plugin_contributes_themes_without_headless_artifacts() {
    if skip_without_runtime() || !tuistory_available() {
        return;
    }

    let sandbox = Sandbox::new("gap-011");
    let run_marker = sandbox.project.join("run-surface-marker.txt");
    write_plugin(
        &sandbox.project.join("tui-plugin.mjs"),
        r#"
import { appendFile } from "node:fs/promises";

export default async function plugin(_input, options) {
  return {
    async event(event) {
      if (event.surface === "run") {
        await appendFile(options.runMarker, "run\n");
      }
    },
    async theme({ register }) {
      register({ id: options.themeID, name: options.themeName });
      return [];
    }
  };
}
"#,
    );
    std::fs::write(
        sandbox.project.join("cortex.toml"),
        format!(
            "plugin = [[\"./tui-plugin.mjs\", {{ type = \"tui\", themeID = \"plugin-neon\", themeName = \"plugin-neon\", runMarker = \"{}\" }}]]\n",
            run_marker.display()
        ),
    )
    .expect("write config");

    let session = format!("gap-011-{}", std::process::id());
    let _guard = launch_tui_session(&sandbox, &session);
    let typed = tuistory(&session, &["type", "/themes"]);
    assert!(typed.status.success(), "tuistory type failed");
    let pressed = tuistory(&session, &["press", "enter"]);
    assert!(pressed.status.success(), "tuistory press enter failed");
    let waited = tuistory(&session, &["wait", "plugin-neon", "--timeout", "20000"]);
    assert!(
        waited.status.success(),
        "tuistory did not find plugin theme:\nstdout={}\nstderr={}",
        String::from_utf8_lossy(&waited.stdout),
        String::from_utf8_lossy(&waited.stderr)
    );
    let snapshot = tuistory(&session, &["snapshot", "--trim"]);
    assert!(
        snapshot.status.success(),
        "tuistory snapshot failed:\nstdout={}\nstderr={}",
        String::from_utf8_lossy(&snapshot.stdout),
        String::from_utf8_lossy(&snapshot.stderr)
    );
    let screen = String::from_utf8_lossy(&snapshot.stdout);
    assert!(
        screen.contains("plugin-neon"),
        "theme picker should include the plugin theme:\n{screen}"
    );

    let output = run(
        cortex(
            &sandbox,
            &["run", "--command", "echo", "--command-arg", "headless-ok"],
        ),
        Duration::from_secs(20),
    );
    assert_eq!(
        output.status.code(),
        Some(0),
        "headless control run should succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        "headless-ok"
    );
    assert!(
        !run_marker.exists(),
        "tui-only plugin must not load on headless run surface"
    );
}

#[test]
fn gap_plugin_012_engine_range_is_enforced() {
    if skip_without_runtime() {
        return;
    }

    let sandbox = Sandbox::new("gap-012");
    write_package_plugin(
        &sandbox.project,
        "gap-engine-range-proof",
        "1.0.0",
        "export default async function plugin() { return { async event() {} }; }",
        ">=99.0.0",
    );
    std::fs::write(
        sandbox.project.join("cortex.toml"),
        "plugin = [\"./index.mjs\"]\n",
    )
    .expect("write config");

    let blocked = run(
        cortex(
            &sandbox,
            &["run", "--command", "echo", "--command-arg", "blocked"],
        ),
        Duration::from_secs(20),
    );
    assert_ne!(blocked.status.code(), Some(0));
    let blocked_stderr = String::from_utf8_lossy(&blocked.stderr);
    assert!(
        blocked_stderr.contains("requires Cortex version"),
        "incompatible engine range should be explicit: {blocked_stderr}"
    );

    write_package_plugin(
        &sandbox.project,
        "gap-engine-range-proof",
        "1.0.0",
        "export default async function plugin() { return { async event() {} }; }",
        ">=0.1.0 <1.0.0",
    );
    let allowed = run(
        cortex(
            &sandbox,
            &["run", "--command", "echo", "--command-arg", "allowed"],
        ),
        Duration::from_secs(20),
    );
    assert_eq!(
        allowed.status.code(),
        Some(0),
        "compatible engine range should load: {}",
        String::from_utf8_lossy(&allowed.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&allowed.stdout).trim(), "allowed");
}

#[test]
fn gap_plugin_013_plugin_install_pins_and_is_deterministic() {
    if skip_without_runtime() {
        return;
    }

    let sandbox = Sandbox::new("gap-013");
    let package = sandbox.project.join("pkg");
    let marker = sandbox.project.join("install-marker.txt");
    write_package_plugin(
        &package,
        "gap-install-proof",
        "2.4.6",
        r#"
import { appendFile } from "node:fs/promises";

export default async function plugin() {
  return {
    async event() {
      await appendFile("install-marker.txt", "loaded\n");
    }
  };
}
"#,
        ">=0.1.0 <1.0.0",
    );

    let spec = format!("file:{}", package.display());
    let install = run(
        cortex(&sandbox, &["plugin", "install", &spec]),
        Duration::from_secs(30),
    );
    assert_eq!(
        install.status.code(),
        Some(0),
        "plugin install should succeed: {}",
        String::from_utf8_lossy(&install.stderr)
    );

    let config_text =
        std::fs::read_to_string(sandbox.project.join("cortex.toml")).expect("project config");
    assert!(
        config_text.contains(&spec),
        "install should record plugin spec in project config: {config_text}"
    );

    let lock_path = sandbox.project.join(".cortex/plugin-lock.json");
    let lock_before = std::fs::read_to_string(&lock_path).expect("plugin lockfile");
    assert!(
        lock_before.contains("gap-install-proof")
            && lock_before.contains("2.4.6")
            && lock_before.contains("sha256:"),
        "lockfile should pin package, version, and integrity: {lock_before}"
    );

    let run_output = run(
        cortex(
            &sandbox,
            &["run", "--command", "echo", "--command-arg", "installed"],
        ),
        Duration::from_secs(20),
    );
    assert_eq!(
        run_output.status.code(),
        Some(0),
        "installed plugin should execute in run surface: {}",
        String::from_utf8_lossy(&run_output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&run_output.stdout).trim(),
        "installed"
    );

    let marker_before_pure = std::fs::read_to_string(&marker).expect("plugin marker");
    let pure_output = run(
        cortex(
            &sandbox,
            &[
                "run",
                "--pure",
                "--command",
                "echo",
                "--command-arg",
                "pure",
            ],
        ),
        Duration::from_secs(20),
    );
    assert_eq!(
        pure_output.status.code(),
        Some(0),
        "--pure run should still execute command: {}",
        String::from_utf8_lossy(&pure_output.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&pure_output.stdout).trim(), "pure");
    let marker_after_pure = std::fs::read_to_string(&marker).expect("plugin marker after pure");
    assert_eq!(
        marker_before_pure, marker_after_pure,
        "--pure should not execute plugin hooks"
    );

    let reinstall = run(
        cortex(&sandbox, &["plug", "install", &spec]),
        Duration::from_secs(30),
    );
    assert_eq!(
        reinstall.status.code(),
        Some(0),
        "reinstall via alias should succeed: {}",
        String::from_utf8_lossy(&reinstall.stderr)
    );
    let lock_after = std::fs::read_to_string(&lock_path).expect("plugin lockfile after reinstall");
    assert_eq!(
        lock_before, lock_after,
        "second install must preserve deterministic lockfile entries"
    );
}
