//! Cross-flow hardening for MCP composition (VAL-GAP-CROSS-010).
//!
//! Proves that MCP roots, resources, prompts, an agent turn, the CLI/server catalogs, the TUI
//! catalog, and the server SSE notification stream all agree for one MCP-backed flow against a real
//! stdio MCP fixture:
//!
//! 1. `cortex mcp list --format json` surfaces the fixture's resource (`notes://demo`) and prompt
//!    (`greet`) with their server-qualified ids.
//! 2. `cortex serve` `GET /mcp` returns the same resource/prompt entries (matching ids, redacted),
//!    and connecting publishes `mcp.resources.changed`/`mcp.prompts.changed` onto the SSE
//!    `GET /event` stream carrying the same uris/names.
//! 3. A live `cortex run` turn uses the `demo_roots_read` tool to resolve a file through the
//!    client-advertised root and the answer is influenced by that fixture file's content.
//! 4. The TUI `/mcp` view lists the same resource (`notes://demo`) and prompt (`greet`) entries.
//!
//! The live turn is gated on a real OpenRouter key and the server/TUI checks are gated on
//! curl/tuistory availability; each skips cleanly when its dependency is absent. Every spawned
//! server is stopped by its lsof-resolved listener PID and stdio fixtures leave no orphan.

use std::io::Read;
use std::net::TcpListener;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use serde_json::Value;

const RUN_TIMEOUT: Duration = Duration::from_secs(120);
const SERVER_START_TIMEOUT: Duration = Duration::from_secs(20);
const ROOTS_MARKER: &str = "CORTEX-ROOTS-PROOF-7Q2X";

static COUNTER: AtomicU32 = AtomicU32::new(0);

fn live_key() -> Option<String> {
    match std::env::var("OPENROUTER_API_KEY") {
        Ok(key) if !key.trim().is_empty() => Some(key),
        _ => None,
    }
}

fn echo_server_bin() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let bin = exe.parent()?.parent()?.join("cortex-mcp-echo-server");
    bin.exists().then_some(bin)
}

fn tuistory_available() -> bool {
    Command::new("tuistory")
        .arg("--version")
        .output()
        .map(|out| out.status.success())
        .unwrap_or(false)
}

struct Sandbox {
    root: PathBuf,
    home: PathBuf,
    project: PathBuf,
    db_path: PathBuf,
    log: PathBuf,
    proc_tag: String,
}

impl Sandbox {
    fn new(tag: &str, echo: &Path, enable_roots: bool) -> Self {
        let unique = format!(
            "{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        );
        let root = std::env::temp_dir().join(format!("cortex-gap-cross-mcp-{tag}-{unique}"));
        let _ = std::fs::remove_dir_all(&root);
        let home = root.join("home");
        let project = root.join("project");
        std::fs::create_dir_all(&home).expect("create home");
        std::fs::create_dir_all(&project).expect("create project");
        let db_path = root.join("cortex.db");
        let log = root.join("mcp.log");

        let roots_line = if enable_roots {
            "CORTEX_MCP_ECHO_ENABLE_ROOTS_TOOL = \"1\"\n"
        } else {
            ""
        };
        // A unique argv token so a global process scan can identify THIS sandbox's stdio fixture
        // without matching sibling tests' identically-named echo-server binaries. The fixture
        // ignores extra argv, so the token only affects how the process appears in `ps`.
        let proc_tag = format!("cortex-mcp-proc-{unique}");
        std::fs::write(
            project.join("cortex.toml"),
            format!(
                r#"
[mcp.demo]
type = "local"
command = ["{}", "{proc_tag}"]
[mcp.demo.environment]
CORTEX_MCP_ECHO_LOG = "{}"
CORTEX_MCP_ECHO_ENABLE_RESOURCES_PROMPTS = "1"
CORTEX_MCP_ECHO_RESOURCE_CONTENT = "{ROOTS_MARKER}"
{roots_line}"#,
                toml_escape(echo),
                toml_escape(&log),
            ),
        )
        .expect("write cortex.toml");

        Self {
            root,
            home,
            project,
            db_path,
            log,
            proc_tag,
        }
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn toml_escape(path: &Path) -> String {
    path.to_string_lossy()
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
}

/// A cortex command rooted in the sandbox project with an isolated HOME and shared store.
fn cortex_command(sandbox: &Sandbox, no_net: bool, args: &[&str]) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_cortex"));
    command.args(args);
    command.current_dir(&sandbox.project);
    command.env("HOME", &sandbox.home);
    command.env(
        "XDG_CONFIG_HOME",
        sandbox.home.join(".config").display().to_string(),
    );
    command.env("CORTEX_DB_PATH", &sandbox.db_path);
    command.env(
        "CORTEX_DATA_DIR",
        sandbox
            .home
            .join(".local/share/cortex")
            .display()
            .to_string(),
    );
    command
        .env_remove("RUST_LOG")
        .env_remove("CORTEX_VERBOSE")
        .env_remove("NO_COLOR");
    if no_net {
        command.env("CORTEX_NO_NET", "1");
    } else {
        command.env_remove("CORTEX_NO_NET");
    }
    command
}

fn spawn_group(mut command: Command) -> Child {
    command.stdin(Stdio::null());
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());
    command.process_group(0);
    command.spawn().expect("spawn process")
}

struct ProcessResult {
    status: Option<ExitStatus>,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

impl ProcessResult {
    fn stdout_str(&self) -> String {
        String::from_utf8_lossy(&self.stdout).into_owned()
    }

    fn stderr_str(&self) -> String {
        String::from_utf8_lossy(&self.stderr).into_owned()
    }

    fn ok(&self) -> bool {
        self.status.and_then(|status| status.code()) == Some(0)
    }
}

fn collect_child(mut child: Child, timeout: Duration) -> ProcessResult {
    let mut stdout = child.stdout.take().expect("stdout");
    let mut stderr = child.stderr.take().expect("stderr");
    let out_thread = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = stdout.read_to_end(&mut bytes);
        bytes
    });
    let err_thread = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = stderr.read_to_end(&mut bytes);
        bytes
    });
    let status = wait_for_exit(&mut child, timeout);
    ProcessResult {
        status,
        stdout: out_thread.join().expect("join stdout"),
        stderr: err_thread.join().expect("join stderr"),
    }
}

fn wait_for_exit(child: &mut Child, timeout: Duration) -> Option<ExitStatus> {
    let start = Instant::now();
    loop {
        if let Some(status) = child.try_wait().expect("try_wait") {
            return Some(status);
        }
        if start.elapsed() > timeout {
            let _ = child.kill();
            let _ = child.wait();
            return None;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn run_to_completion(command: Command, timeout: Duration) -> ProcessResult {
    collect_child(spawn_group(command), timeout)
}

fn free_server_port() -> u16 {
    for port in 3260..=3299 {
        if TcpListener::bind(("127.0.0.1", port)).is_ok() {
            return port;
        }
    }
    panic!("no free server port in 3260-3299");
}

fn curl(args: &[&str]) -> ProcessResult {
    let mut command = Command::new("curl");
    command.args(args);
    run_to_completion(command, Duration::from_secs(15))
}

fn curl_available() -> bool {
    Command::new("curl")
        .arg("--version")
        .output()
        .map(|out| out.status.success())
        .unwrap_or(false)
}

fn listener_pid(port: u16) -> Option<u32> {
    let output = Command::new("lsof")
        .args(["-tiTCP", &port.to_string(), "-sTCP:LISTEN"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .find_map(|line| line.trim().parse::<u32>().ok())
}

fn signal_pid(sig: &str, pid: u32) -> bool {
    Command::new("kill")
        .args([sig, &pid.to_string()])
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

fn wait_for_health(port: u16) {
    let started = Instant::now();
    loop {
        let response = curl(&["-sf", &format!("http://127.0.0.1:{port}/health")]);
        if response.ok() {
            return;
        }
        assert!(
            started.elapsed() < SERVER_START_TIMEOUT,
            "server on port {port} did not become healthy, last stderr: {}",
            response.stderr_str()
        );
        std::thread::sleep(Duration::from_millis(100));
    }
}

struct ServerGuard {
    child: Option<Child>,
    port: u16,
}

impl ServerGuard {
    fn stop(&mut self) {
        let Some(mut child) = self.child.take() else {
            return;
        };
        let listener = listener_pid(self.port).unwrap_or_else(|| child.id());
        let _ = signal_pid("-TERM", listener);
        if wait_for_exit(&mut child, Duration::from_secs(10)).is_none() {
            let _ = signal_pid("-KILL", listener);
            let _ = child.wait();
        }
        if let Some(mut stdout) = child.stdout.take() {
            let _ = stdout.read_to_end(&mut Vec::new());
        }
        if let Some(mut stderr) = child.stderr.take() {
            let _ = stderr.read_to_end(&mut Vec::new());
        }
    }
}

impl Drop for ServerGuard {
    fn drop(&mut self) {
        self.stop();
    }
}

// ---- (1) CLI catalog ----

#[test]
fn gap_cross_010_cli_mcp_list_surfaces_resources_and_prompts() {
    let Some(echo) = echo_server_bin() else {
        eprintln!("skipping: cortex-mcp-echo-server fixture is unavailable");
        return;
    };
    let sandbox = Sandbox::new("cli", &echo, true);

    let result = run_to_completion(
        cortex_command(&sandbox, false, &["mcp", "list", "--format", "json"]),
        Duration::from_secs(30),
    );
    assert!(
        result.ok(),
        "cortex mcp list should succeed; stderr: {}",
        result.stderr_str()
    );
    let document: Value = serde_json::from_str(result.stdout_str().trim())
        .expect("cortex mcp list emits a single JSON document");
    let demo = document["servers"]
        .as_array()
        .and_then(|servers| servers.iter().find(|server| server["name"] == "demo"))
        .expect("demo server present in mcp list");
    assert_eq!(demo["status"], "connected");

    let resource = demo["resources"]
        .as_array()
        .and_then(|resources| resources.iter().find(|r| r["uri"] == "notes://demo"))
        .expect("notes://demo resource listed");
    assert!(
        resource["id"].as_str().is_some_and(|id| !id.is_empty()),
        "resource carries a server-qualified id: {demo:?}"
    );

    let prompt = demo["prompts"]
        .as_array()
        .and_then(|prompts| prompts.iter().find(|p| p["name"] == "greet"))
        .expect("greet prompt listed");
    assert!(
        prompt["id"].as_str().is_some_and(|id| !id.is_empty()),
        "prompt carries a server-qualified id: {demo:?}"
    );
}

// ---- (2) Server catalog + SSE ----

#[test]
fn gap_cross_010_server_mcp_catalog_and_sse_agree() {
    let Some(echo) = echo_server_bin() else {
        eprintln!("skipping: cortex-mcp-echo-server fixture is unavailable");
        return;
    };
    if !curl_available() {
        eprintln!("skipping: curl is unavailable");
        return;
    }
    let sandbox = Sandbox::new("server", &echo, true);
    let port = free_server_port();

    let serve = cortex_command(
        &sandbox,
        true,
        &[
            "serve",
            "--hostname",
            "127.0.0.1",
            "--port",
            &port.to_string(),
        ],
    );
    let mut server = ServerGuard {
        child: Some(spawn_group(serve)),
        port,
    };
    wait_for_health(port);

    // Start an SSE subscriber that will capture the catalogue-change notifications, then trigger a
    // /mcp connect that publishes them.
    let sse_path = sandbox.root.join("sse.out");
    let sse_file = std::fs::File::create(&sse_path).expect("create sse capture file");
    let mut sse = Command::new("curl")
        .args([
            "-sN",
            "--max-time",
            "8",
            &format!("http://127.0.0.1:{port}/event"),
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::from(sse_file))
        .stderr(Stdio::null())
        .process_group(0)
        .spawn()
        .expect("spawn sse curl");
    std::thread::sleep(Duration::from_millis(700));

    let catalog = curl(&["-sf", &format!("http://127.0.0.1:{port}/mcp")]);
    assert!(
        catalog.ok(),
        "GET /mcp should succeed; stderr: {}",
        catalog.stderr_str()
    );
    let document: Value =
        serde_json::from_str(catalog.stdout_str().trim()).expect("GET /mcp emits JSON");

    let server_resource = document["resources"]
        .as_array()
        .and_then(|resources| resources.iter().find(|r| r["uri"] == "notes://demo"))
        .expect("server /mcp lists notes://demo");
    let server_resource_id = server_resource["id"]
        .as_str()
        .expect("server resource id")
        .to_owned();
    let server_prompt = document["prompts"]
        .as_array()
        .and_then(|prompts| prompts.iter().find(|p| p["name"] == "greet"))
        .expect("server /mcp lists greet");
    let server_prompt_id = server_prompt["id"]
        .as_str()
        .expect("server prompt id")
        .to_owned();

    // The server resource/prompt ids must match what the CLI surface reports for the same fixture.
    let cli = run_to_completion(
        cortex_command(&sandbox, false, &["mcp", "list", "--format", "json"]),
        Duration::from_secs(30),
    );
    assert!(cli.ok(), "cortex mcp list stderr: {}", cli.stderr_str());
    let cli_document: Value = serde_json::from_str(cli.stdout_str().trim()).expect("mcp list JSON");
    let cli_demo = &cli_document["servers"]
        .as_array()
        .and_then(|servers| servers.iter().find(|server| server["name"] == "demo"))
        .expect("cli demo server")
        .clone();
    let cli_resource_id = cli_demo["resources"][0]["id"].as_str().unwrap_or_default();
    let cli_prompt_id = cli_demo["prompts"]
        .as_array()
        .and_then(|prompts| prompts.iter().find(|p| p["name"] == "greet"))
        .and_then(|p| p["id"].as_str())
        .unwrap_or_default();
    assert_eq!(
        server_resource_id, cli_resource_id,
        "server and CLI must agree on the resource id"
    );
    assert_eq!(
        server_prompt_id, cli_prompt_id,
        "server and CLI must agree on the prompt id"
    );

    // The SSE stream must carry the same resource/prompt entries the catalog returned.
    let mut sse_text;
    let started = Instant::now();
    loop {
        sse_text = std::fs::read_to_string(&sse_path).unwrap_or_default();
        let has_resource =
            sse_text.contains("mcp.resources.changed") && sse_text.contains("notes://demo");
        let has_prompt = sse_text.contains("mcp.prompts.changed") && sse_text.contains("greet");
        if has_resource && has_prompt {
            break;
        }
        if started.elapsed() > Duration::from_secs(10) {
            break;
        }
        std::thread::sleep(Duration::from_millis(150));
    }
    let _ = sse.kill();
    let _ = sse.wait();

    assert!(
        sse_text.contains("mcp.resources.changed") && sse_text.contains("notes://demo"),
        "SSE should announce the demo resource; got:\n{sse_text}"
    );
    assert!(
        sse_text.contains("mcp.prompts.changed") && sse_text.contains("greet"),
        "SSE should announce the demo prompt; got:\n{sse_text}"
    );

    server.stop();
    assert!(
        TcpListener::bind(("127.0.0.1", port)).is_ok(),
        "server port {port} should be released after stopping"
    );
}

// ---- (3) Live turn resolves a resource through roots ----

#[test]
fn gap_cross_010_live_turn_resolves_resource_through_roots() {
    let Some(key) = live_key() else {
        eprintln!("skipping live test: OPENROUTER_API_KEY not set");
        return;
    };
    let Some(echo) = echo_server_bin() else {
        eprintln!("skipping: cortex-mcp-echo-server fixture is unavailable");
        return;
    };
    let sandbox = Sandbox::new("live", &echo, true);
    // The marker file lives under the project directory, which `cortex run` advertises as the MCP
    // client root, so the server can only read it by resolving the path through roots/list.
    std::fs::write(sandbox.project.join("marker.txt"), ROOTS_MARKER).expect("write marker file");

    let mut command = cortex_command(
        &sandbox,
        false,
        &[
            "run",
            "--jsonl",
            "Call the demo_roots_read tool with the argument path set to \"marker.txt\". \
             Then reply with the exact secret token that file contains and nothing else.",
        ],
    );
    command.env("OPENROUTER_API_KEY", key);
    let result = run_to_completion(command, RUN_TIMEOUT);
    let stdout = result.stdout_str();
    assert!(
        result.ok(),
        "cortex run should succeed; stdout: {stdout}\nstderr: {}",
        result.stderr_str()
    );

    // The fixture log proves the tool call reached the server via roots resolution.
    let log = std::fs::read_to_string(&sandbox.log).unwrap_or_default();
    assert!(
        log.contains("tools/call name=roots_read"),
        "the fixture should have received the roots_read tool call; log:\n{log}"
    );

    // The fixture content resolved through roots must influence the turn output (tool result and/or
    // the final answer carry the unique marker token).
    assert!(
        stdout.contains(ROOTS_MARKER),
        "the answer should be influenced by the root-resolved fixture content ({ROOTS_MARKER}); \
         stdout:\n{stdout}"
    );

    // No orphaned stdio MCP child should survive the run. Scope the scan to this sandbox's unique
    // argv tag so concurrent sibling tests' fixtures are never misattributed, and poll briefly
    // because the OS may take a moment to reap the killed child.
    let proc_tag = sandbox.proc_tag.clone();
    let mut leaked = true;
    for _ in 0..30 {
        let alive = Command::new("ps")
            .args(["-eo", "stat,args"])
            .output()
            .map(|out| {
                String::from_utf8_lossy(&out.stdout)
                    .lines()
                    .filter(|line| line.contains(proc_tag.as_str()))
                    .any(|line| {
                        !line
                            .split_whitespace()
                            .next()
                            .is_some_and(|stat| stat.starts_with('Z'))
                    })
            })
            .unwrap_or(false);
        if !alive {
            leaked = false;
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(!leaked, "cortex run must reap the stdio MCP fixture by PID");
}

// ---- (4) TUI catalog ----

#[test]
fn gap_cross_010_tui_mcp_view_lists_resources_and_prompts() {
    let Some(echo) = echo_server_bin() else {
        eprintln!("skipping: cortex-mcp-echo-server fixture is unavailable");
        return;
    };
    if !tuistory_available() {
        eprintln!("skipping TUI MCP view test: tuistory is unavailable");
        return;
    }
    let sandbox = Sandbox::new("tui", &echo, true);

    let session = format!(
        "gap-cross-010-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    );
    let _guard = TuiSession::launch(&sandbox, &session);

    // Open the MCP status view and confirm it lists the same resource and prompt entries.
    assert!(tuistory(&session, &["type", "/mcp"]).status.success());
    assert!(tuistory(&session, &["press", "enter"]).status.success());

    assert!(
        wait_for_snapshot(&session, "notes://demo", Duration::from_secs(20)),
        "the TUI /mcp view should list the MCP resource uri"
    );
    assert!(
        wait_for_snapshot(&session, "greet", Duration::from_secs(20)),
        "the TUI /mcp view should list the MCP prompt name"
    );
}

struct TuiSession {
    session: String,
}

impl TuiSession {
    fn launch(sandbox: &Sandbox, session: &str) -> Self {
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
            .arg(format!("CORTEX_DB_PATH={}", sandbox.db_path.display()))
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
        Self {
            session: session.to_owned(),
        }
    }
}

impl Drop for TuiSession {
    fn drop(&mut self) {
        let _ = Command::new("tuistory")
            .arg("-s")
            .arg(&self.session)
            .arg("close")
            .output();
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

fn wait_for_snapshot(session: &str, needle: &str, timeout: Duration) -> bool {
    let started = Instant::now();
    while started.elapsed() < timeout {
        let output = tuistory(session, &["snapshot", "--immediate", "--trim"]);
        if output.status.success() && String::from_utf8_lossy(&output.stdout).contains(needle) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(150));
    }
    false
}
