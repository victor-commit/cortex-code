use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

struct Sandbox {
    root: PathBuf,
    home: PathBuf,
    project: PathBuf,
}

impl Sandbox {
    fn new(tag: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "cortex-gap-tui-mcp-prompt-slash-{tag}-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&root);
        let home = root.join("home");
        let project = root.join("project");
        std::fs::create_dir_all(&home).expect("create home");
        std::fs::create_dir_all(&project).expect("create project");
        Self {
            root,
            home,
            project,
        }
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn write(path: impl AsRef<Path>, body: &str) {
    std::fs::write(path, body).expect("write fixture");
}

fn toml_string(value: &Path) -> String {
    value
        .to_string_lossy()
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
}

fn tuistory_available() -> bool {
    Command::new("tuistory")
        .arg("--version")
        .output()
        .map(|out| out.status.success())
        .unwrap_or(false)
}

fn echo_server_bin() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let bin = exe.parent()?.parent()?.join("cortex-mcp-echo-server");
    bin.exists().then_some(bin)
}

struct SessionGuard {
    session: String,
}

impl Drop for SessionGuard {
    fn drop(&mut self) {
        let _ = Command::new("tuistory")
            .arg("-s")
            .arg(&self.session)
            .arg("close")
            .output();
    }
}

fn launch_tui_session(sandbox: &Sandbox, session: &str) -> SessionGuard {
    let output = Command::new("tuistory")
        .arg("launch")
        .arg("--session")
        .arg(session)
        .arg("--background")
        .arg("--cols")
        .arg("240")
        .arg("--rows")
        .arg("60")
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
        .arg("--env")
        .arg("OPENROUTER_API_KEY=")
        .arg(format!("{} tui", env!("CARGO_BIN_EXE_cortex")))
        .output()
        .expect("launch tuistory session");
    assert!(
        output.status.success(),
        "tuistory launch failed:\nstdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    SessionGuard {
        session: session.to_owned(),
    }
}

fn tuistory(session: &str, args: &[&str]) -> std::process::Output {
    Command::new("tuistory")
        .arg("-s")
        .arg(session)
        .args(args)
        .output()
        .expect("run tuistory command")
}

fn snapshot(session: &str) -> String {
    let output = tuistory(session, &["snapshot", "--immediate", "--trim"]);
    assert!(
        output.status.success(),
        "snapshot failed:\nstdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn wait_for_contains(session: &str, needle: &str, timeout: Duration) -> bool {
    let started = Instant::now();
    while started.elapsed() < timeout {
        if snapshot(session).contains(needle) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(120));
    }
    false
}

fn wait_for_log_contains(path: &Path, needle: &str, timeout: Duration) -> bool {
    let started = Instant::now();
    while started.elapsed() < timeout {
        if let Ok(contents) = std::fs::read_to_string(path)
            && contents.contains(needle)
        {
            return true;
        }
        std::thread::sleep(Duration::from_millis(120));
    }
    false
}

#[test]
fn gap_tui_008_mcp_prompt_slash_fixture_covers_prompt_commands() {
    if !tuistory_available() {
        eprintln!("skipping TUI MCP prompt slash test: tuistory is unavailable");
        return;
    }
    let Some(echo_server) = echo_server_bin() else {
        eprintln!(
            "skipping TUI MCP prompt slash test: cortex-mcp-echo-server fixture is unavailable"
        );
        return;
    };

    let sandbox = Sandbox::new("val-gap-tui-008");
    let log_path = sandbox.root.join("mcp.log");
    write(
        sandbox.project.join("cortex.toml"),
        &format!(
            r#"
[mcp.demo]
type = "local"
command = ["{}"]
[mcp.demo.environment]
CORTEX_MCP_ECHO_LOG = "{}"
CORTEX_MCP_ECHO_ENABLE_RESOURCES_PROMPTS = "1"
CORTEX_MCP_ECHO_ENABLE_NOARGS_PROMPT = "1"
"#,
            toml_string(&echo_server),
            toml_string(&log_path)
        ),
    );

    let session = format!(
        "gap-tui-008-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    );
    let _guard = launch_tui_session(&sandbox, &session);

    assert!(
        tuistory(&session, &["type", "/demo_prompt_"])
            .status
            .success()
    );
    assert!(
        wait_for_contains(&session, "demo_prompt_ping", Duration::from_secs(20)),
        "slash autocomplete should include the no-args MCP prompt command"
    );
    assert!(
        wait_for_contains(&session, "demo_prompt_greet", Duration::from_secs(20)),
        "slash autocomplete should include the required-args MCP prompt command"
    );

    assert!(tuistory(&session, &["press", "esc"]).status.success());
    assert!(
        tuistory(&session, &["type", "/demo_prompt_greet"])
            .status
            .success()
    );
    assert!(tuistory(&session, &["press", "enter"]).status.success());
    assert!(
        wait_for_contains(
            &session,
            "requires arguments (name)",
            Duration::from_secs(15)
        ),
        "required MCP prompts should report a clear unsupported-required-args diagnostic"
    );

    assert!(
        tuistory(&session, &["type", "/demo_prompt_ping"])
            .status
            .success()
    );
    assert!(tuistory(&session, &["press", "enter"]).status.success());
    assert!(
        wait_for_contains(&session, "running MCP prompt", Duration::from_secs(15)),
        "executing a no-args MCP slash prompt should surface running prompt/dialog state"
    );
    assert!(
        wait_for_log_contains(&log_path, "prompts/get name=ping", Duration::from_secs(15)),
        "fixture log should confirm slash execution reached prompts/get"
    );

    let log_contents = std::fs::read_to_string(&log_path).expect("read MCP fixture log");
    assert!(
        !log_contents.contains("prompts/get name=greet"),
        "required-args prompt command must not call prompts/get before args are supported"
    );
}
