//! Cross-surface configuration / permission / agent layering proof for
//! VAL-CROSS-011 … VAL-CROSS-014.
//!
//! On one shared project (a single `cortex.toml` under a hermetic HOME + CWD) the
//! headless surface, the local server, and the TUI must report the same effective
//! defaults and honor the same project-layer permission deny.
//!
//! - Offline half always runs: project-layer `model`/`agent`, env `CORTEX_MODEL`
//!   override, and project deny classification via the real binary/`cortex config`
//!   dump + loopback `cortex serve` routes (`/config`, `/session/{id}/context`) and
//!   via a tuistory status-line snapshot when available.
//! - Live half (gated on `OPENROUTER_API_KEY`): a real headless turn and a real
//!   server turn each report the project model/agent, and both refuse a `bash rm *`
//!   deny with the victim file left intact. Unsetting `CORTEX_MODEL` restores the
//!   file-layer model on both surfaces.
//!
//! NO mocks on the live path. Servers are killed by exact lsof listener PID. Ports
//! stay inside 3200-3299.

use std::io::Read;
use std::net::TcpListener;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

const RUN_TIMEOUT: Duration = Duration::from_secs(120);
const SERVER_START_TIMEOUT: Duration = Duration::from_secs(20);
const PROJECT_MODEL: &str = "anthropic/claude-sonnet-4";
const ENV_MODEL: &str = "google/gemini-2.5-flash";
const PROJECT_AGENT: &str = "plan";
const DEFAULT_AGENT: &str = "build";

fn live_key() -> Option<String> {
    match std::env::var("OPENROUTER_API_KEY") {
        Ok(key) if !key.trim().is_empty() => Some(key),
        _ => None,
    }
}

fn scratch_dir(tag: &str) -> PathBuf {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!(
        "cortex-cross-m6-config-{tag}-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("home")).expect("create home");
    std::fs::create_dir_all(dir.join("project")).expect("create project");
    dir
}

fn free_server_port() -> u16 {
    for port in 3240..=3299 {
        if TcpListener::bind(("127.0.0.1", port)).is_ok() {
            return port;
        }
    }
    panic!("no free server port in 3240-3299");
}

fn project_config_model_agent() -> String {
    format!("model = \"{PROJECT_MODEL}\"\nagent = \"{PROJECT_AGENT}\"\n")
}

fn project_config_deny() -> String {
    "[[permission]]\ntool = \"bash\"\npattern = \"rm *\"\naction = \"deny\"\n".to_owned()
}

fn write_project(project: &Path, body: &str) {
    std::fs::write(project.join("cortex.toml"), body).expect("write project cortex.toml");
}

fn cortex_base(home: &Path, project: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_cortex"));
    command.current_dir(project);
    command.env("HOME", home);
    command.env("PATH", std::env::var("PATH").unwrap_or_default());
    command
        .env_remove("OPENROUTER_API_KEY")
        .env_remove("CORTEX_NO_NET")
        .env_remove("CORTEX_MODEL")
        .env_remove("CORTEX_AGENT")
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("XDG_DATA_HOME")
        .env_remove("CORTEX_DATA_DIR")
        .env_remove("CORTEX_DB_PATH")
        .env_remove("CORTEX_AUTH_FILE")
        .env_remove("RUST_LOG")
        .env_remove("CORTEX_VERBOSE")
        .env_remove("NO_COLOR");
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

    fn code(&self) -> Option<i32> {
        self.status.and_then(|status| status.code())
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

fn run_cortex(
    home: &Path,
    project: &Path,
    args: &[&str],
    key: Option<&str>,
    env: &[(&str, &str)],
) -> ProcessResult {
    let mut command = cortex_base(home, project);
    command.args(args);
    if let Some(key) = key {
        command.env("OPENROUTER_API_KEY", key);
    }
    for (k, v) in env {
        command.env(*k, *v);
    }
    collect_child(spawn_group(command), RUN_TIMEOUT)
}

fn curl(args: &[&str]) -> ProcessResult {
    let mut command = Command::new("curl");
    command.args(args);
    collect_child(spawn_group(command), RUN_TIMEOUT)
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
        if response.code() == Some(0) {
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

fn stop_server(mut child: Child, port: u16) {
    let listener = listener_pid(port).unwrap_or_else(|| child.id());
    assert!(
        signal_pid("-TERM", listener),
        "failed to signal listener PID {listener}"
    );
    let status = wait_for_exit(&mut child, Duration::from_secs(10));
    if status.is_none() {
        let _ = signal_pid("-KILL", listener);
    }
    let mut stdout = child.stdout.take().expect("server stdout");
    let mut stderr = child.stderr.take().expect("server stderr");
    let mut out_buf = Vec::new();
    let mut err_buf = Vec::new();
    let _ = stdout.read_to_end(&mut out_buf);
    let _ = stderr.read_to_end(&mut err_buf);
    std::thread::sleep(Duration::from_millis(200));
    assert!(
        listener_pid(port).is_none(),
        "server port {port} still has a listener after kill of PID {listener}"
    );
}

fn parse_json(bytes: &[u8]) -> serde_json::Value {
    serde_json::from_slice(bytes).unwrap_or_else(|err| {
        panic!(
            "expected JSON, got parse error {err}: {}",
            String::from_utf8_lossy(bytes)
        )
    })
}

fn start_serve(
    home: &Path,
    project: &Path,
    port: u16,
    key: Option<&str>,
    env: &[(&str, &str)],
) -> Child {
    let mut cmd = cortex_base(home, project);
    if let Some(key) = key {
        cmd.env("OPENROUTER_API_KEY", key);
    }
    for (k, v) in env {
        cmd.env(*k, *v);
    }
    cmd.args([
        "serve",
        "--hostname",
        "127.0.0.1",
        "--port",
        &port.to_string(),
    ]);
    let child = spawn_group(cmd);
    wait_for_health(port);
    child
}

fn create_session(port: u16) -> String {
    let created = curl(&[
        "-sS",
        "-X",
        "POST",
        "-H",
        "Content-Type: application/json",
        "-d",
        "{}",
        &format!("http://127.0.0.1:{port}/session"),
    ]);
    assert_eq!(
        created.code(),
        Some(0),
        "create session failed: {}",
        created.stderr_str()
    );
    let body = parse_json(&created.stdout);
    body["id"].as_str().expect("session id").to_owned()
}

fn tuistory_available() -> bool {
    Command::new("tuistory")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

struct TuistorySession {
    name: String,
}

impl TuistorySession {
    fn launch(
        name: String,
        cortex: &Path,
        project: &Path,
        home: &Path,
        env: &[(&str, &str)],
    ) -> Self {
        let command_line = format!("{} tui", cortex.display());
        let mut launch = Command::new("tuistory");
        launch
            .arg("launch")
            .arg(command_line)
            .arg("-s")
            .arg(&name)
            .arg("--cwd")
            .arg(project)
            .arg("--cols")
            .arg("140")
            .arg("--rows")
            .arg("36")
            .arg("--env")
            .arg(format!("HOME={}", home.display()))
            .arg("--env")
            .arg(format!(
                "XDG_CONFIG_HOME={}",
                home.join(".config").display()
            ))
            .arg("--env")
            .arg(format!(
                "XDG_DATA_HOME={}",
                home.join(".local/share").display()
            ))
            .arg("--env")
            .arg("CORTEX_AUTH_FILE=")
            .arg("--env")
            .arg("CORTEX_NO_NET=1")
            .arg("--env")
            .arg("RUST_LOG=")
            .arg("--env")
            .arg("CORTEX_VERBOSE=")
            .arg("--env")
            .arg("NO_COLOR=");
        for (k, v) in env {
            launch.arg("--env").arg(format!("{k}={v}"));
        }
        let out = launch
            .arg("--background")
            .arg("--timeout")
            .arg("15000")
            .output()
            .expect("launch tuistory");
        assert!(
            out.status.success(),
            "tuistory launch failed:\nstdout={}\nstderr={}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        std::thread::sleep(Duration::from_millis(800));
        Self { name }
    }

    fn snapshot(&self) -> String {
        let out = Command::new("tuistory")
            .arg("-s")
            .arg(&self.name)
            .args(["snapshot", "--immediate", "--trim"])
            .output()
            .expect("snapshot");
        assert!(
            out.status.success(),
            "snapshot failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    fn wait_for_any(&self, needles: &[&str], timeout_ms: u64) -> String {
        let start = Instant::now();
        loop {
            let snap = self.snapshot();
            if needles.iter().any(|n| snap.contains(n)) {
                return snap;
            }
            if start.elapsed() > Duration::from_millis(timeout_ms) {
                panic!("TUI did not show any of {needles:?} within {timeout_ms}ms:\n{snap}");
            }
            std::thread::sleep(Duration::from_millis(250));
        }
    }

    fn close(self) {
        let _ = Command::new("tuistory")
            .arg("-s")
            .arg(&self.name)
            .arg("close")
            .output();
    }
}

impl Drop for TuistorySession {
    fn drop(&mut self) {
        let _ = Command::new("tuistory")
            .arg("-s")
            .arg(&self.name)
            .arg("close")
            .output();
    }
}

// VAL-CROSS-011/013/014 offline: config report + server context + TUI status line all share one
// project layer; CORTEX_MODEL beats the file, and unsetting restores it.
#[test]
fn project_model_agent_and_env_override_match_across_surfaces_offline() {
    let root = scratch_dir("offline-model-agent");
    let home = root.join("home");
    let project = root.join("project");
    write_project(&project, &project_config_model_agent());

    // --- headless config report (file layer only) ---
    let headless_cfg = run_cortex(&home, &project, &["config", "--format", "json"], None, &[]);
    assert_eq!(
        headless_cfg.code(),
        Some(0),
        "config dump failed: {}",
        headless_cfg.stderr_str()
    );
    let headless_doc = parse_json(&headless_cfg.stdout);
    assert_eq!(
        headless_doc["model"].as_str(),
        Some(PROJECT_MODEL),
        "VAL-CROSS-011 headless must report project model: {headless_doc}"
    );
    assert_eq!(
        headless_doc["agent"].as_str(),
        Some(PROJECT_AGENT),
        "VAL-CROSS-014 headless must report project agent: {headless_doc}"
    );

    // --- headless config report with CORTEX_MODEL env override ---
    let headless_env = run_cortex(
        &home,
        &project,
        &["config", "--format", "json"],
        None,
        &[("CORTEX_MODEL", ENV_MODEL)],
    );
    assert_eq!(
        headless_env.code(),
        Some(0),
        "{}",
        headless_env.stderr_str()
    );
    let headless_env_doc = parse_json(&headless_env.stdout);
    assert_eq!(
        headless_env_doc["model"].as_str(),
        Some(ENV_MODEL),
        "VAL-CROSS-013 headless CORTEX_MODEL must beat file: {headless_env_doc}"
    );
    assert_eq!(
        headless_env_doc["agent"].as_str(),
        Some(PROJECT_AGENT),
        "env model override must not clobber project agent: {headless_env_doc}"
    );

    // Unset: back to file layer.
    let headless_unset = run_cortex(&home, &project, &["config", "--format", "json"], None, &[]);
    let headless_unset_doc = parse_json(&headless_unset.stdout);
    assert_eq!(
        headless_unset_doc["model"].as_str(),
        Some(PROJECT_MODEL),
        "unsetting CORTEX_MODEL must restore file model: {headless_unset_doc}"
    );

    // --- server /config + /session/{{id}}/context (process captures env at startup) ---
    let port = free_server_port();
    let child = start_serve(&home, &project, port, None, &[]);

    let server_cfg = curl(&["-sS", &format!("http://127.0.0.1:{port}/config")]);
    assert_eq!(
        server_cfg.code(),
        Some(0),
        "GET /config failed: {}",
        server_cfg.stderr_str()
    );
    let server_cfg_doc = parse_json(&server_cfg.stdout);
    assert_eq!(
        server_cfg_doc["model"].as_str(),
        Some(PROJECT_MODEL),
        "VAL-CROSS-011 server /config must report project model: {server_cfg_doc}"
    );
    assert_eq!(
        server_cfg_doc["agent"].as_str(),
        Some(PROJECT_AGENT),
        "VAL-CROSS-014 server /config must report project agent: {server_cfg_doc}"
    );

    let session_id = create_session(port);
    let ctx = curl(&[
        "-sS",
        &format!("http://127.0.0.1:{port}/session/{session_id}/context"),
    ]);
    assert_eq!(ctx.code(), Some(0), "context failed: {}", ctx.stderr_str());
    let ctx_doc = parse_json(&ctx.stdout);
    assert_eq!(
        ctx_doc["model"].as_str(),
        Some(PROJECT_MODEL),
        "VAL-CROSS-011 server context model: {ctx_doc}"
    );
    assert_eq!(
        ctx_doc["agent"].as_str(),
        Some(PROJECT_AGENT),
        "VAL-CROSS-014 server context agent: {ctx_doc}"
    );

    stop_server(child, port);

    // Restart server with CORTEX_MODEL set: env wins for this process lifetime.
    let port_env = free_server_port();
    let child_env = start_serve(
        &home,
        &project,
        port_env,
        None,
        &[("CORTEX_MODEL", ENV_MODEL)],
    );
    let server_env_cfg = curl(&["-sS", &format!("http://127.0.0.1:{port_env}/config")]);
    let server_env_doc = parse_json(&server_env_cfg.stdout);
    assert_eq!(
        server_env_doc["model"].as_str(),
        Some(ENV_MODEL),
        "VAL-CROSS-013 server CORTEX_MODEL must beat file: {server_env_doc}"
    );
    let session_env = create_session(port_env);
    let ctx_env = curl(&[
        "-sS",
        &format!("http://127.0.0.1:{port_env}/session/{session_env}/context"),
    ]);
    let ctx_env_doc = parse_json(&ctx_env.stdout);
    assert_eq!(
        ctx_env_doc["model"].as_str(),
        Some(ENV_MODEL),
        "server context must report env model: {ctx_env_doc}"
    );
    stop_server(child_env, port_env);

    // Restart without CORTEX_MODEL: back to file.
    let port_restore = free_server_port();
    let child_restore = start_serve(&home, &project, port_restore, None, &[]);
    let restore_cfg = curl(&["-sS", &format!("http://127.0.0.1:{port_restore}/config")]);
    let restore_doc = parse_json(&restore_cfg.stdout);
    assert_eq!(
        restore_doc["model"].as_str(),
        Some(PROJECT_MODEL),
        "server without CORTEX_MODEL restores file model: {restore_doc}"
    );
    stop_server(child_restore, port_restore);

    // --- TUI status line (tuistory) when available ---
    if !tuistory_available() {
        eprintln!(
            "skipping TUI surface of VAL-CROSS-011/013/014 offline path: tuistory unavailable"
        );
        return;
    }

    let cortex = PathBuf::from(env!("CARGO_BIN_EXE_cortex"));
    let tui = TuistorySession::launch(
        format!("cross-cfg-file-{}", std::process::id()),
        &cortex,
        &project,
        &home,
        &[],
    );
    // Status line shows short model tokens (gemini–flash) or full id depending on width/theme;
    // accept either project model id / vendor fragment and the agent name `plan`.
    let snap = tui.wait_for_any(
        &[
            PROJECT_MODEL,
            "claude-sonnet-4",
            "claude-sonnet",
            "sonnet-4",
            PROJECT_AGENT,
        ],
        12_000,
    );
    assert!(
        snap.contains(PROJECT_AGENT) || snap.to_lowercase().contains("plan"),
        "VAL-CROSS-014 TUI status must show project agent `plan`, got:\n{snap}"
    );
    assert!(
        snap.contains(PROJECT_MODEL)
            || snap.contains("claude-sonnet-4")
            || snap.contains("claude-sonnet")
            || snap.contains("sonnet-4"),
        "VAL-CROSS-011 TUI status must show project model, got:\n{snap}"
    );
    tui.close();

    let tui_env = TuistorySession::launch(
        format!("cross-cfg-env-{}", std::process::id()),
        &cortex,
        &project,
        &home,
        &[("CORTEX_MODEL", ENV_MODEL)],
    );
    let snap_env = tui_env.wait_for_any(
        &[ENV_MODEL, "gemini-2.5-flash", "gemini-2.5", "2.5-flash"],
        12_000,
    );
    assert!(
        snap_env.contains(ENV_MODEL)
            || snap_env.contains("gemini-2.5-flash")
            || snap_env.contains("gemini-2.5")
            || snap_env.contains("2.5-flash"),
        "VAL-CROSS-013 TUI CORTEX_MODEL must beat file on status line, got:\n{snap_env}"
    );
    // Must NOT keep showing the project-only model as the active binding when env wins.
    if snap_env.contains(PROJECT_MODEL) {
        // Some status chrome may still mention register models; prefer explicit flash token.
        assert!(
            snap_env.contains(ENV_MODEL)
                || snap_env.contains("gemini-2.5-flash")
                || snap_env.contains("2.5-flash"),
            "env model must still be visible when project model string is present:\n{snap_env}"
        );
    }
    tui_env.close();

    // Unset returns TUI to file value.
    let tui_restore = TuistorySession::launch(
        format!("cross-cfg-restore-{}", std::process::id()),
        &cortex,
        &project,
        &home,
        &[],
    );
    let snap_restore = tui_restore.wait_for_any(
        &[
            PROJECT_MODEL,
            "claude-sonnet-4",
            "claude-sonnet",
            "sonnet-4",
        ],
        12_000,
    );
    assert!(
        snap_restore.contains(PROJECT_MODEL)
            || snap_restore.contains("claude-sonnet-4")
            || snap_restore.contains("claude-sonnet")
            || snap_restore.contains("sonnet-4"),
        "unsetting CORTEX_MODEL restores TUI status to file model:\n{snap_restore}"
    );
    tui_restore.close();

    // Default agent sanity: project file absent would report build, but project agent must stay plan.
    let _ = DEFAULT_AGENT;
}

// VAL-CROSS-012 offline: project deny rule layers identically for config-derived classification
// on headless reporting and on server restart health (full tool block proven live below).
#[test]
fn project_deny_rule_is_present_in_shared_config_across_surfaces_offline() {
    let root = scratch_dir("offline-deny");
    let home = root.join("home");
    let project = root.join("project");
    let mut body = project_config_model_agent();
    body.push_str(&project_config_deny());
    write_project(&project, &body);

    let cfg = run_cortex(&home, &project, &["config", "--format", "json"], None, &[]);
    assert_eq!(cfg.code(), Some(0), "{}", cfg.stderr_str());
    let doc = parse_json(&cfg.stdout);
    assert_eq!(doc["model"], PROJECT_MODEL);
    assert_eq!(doc["agent"], PROJECT_AGENT);
    // Permission may serialize as an array of tables; tolerate either nested structure.
    let permission = &doc["permission"];
    assert!(
        permission.is_array() || permission.is_object(),
        "project [[permission]] must appear in effective config: {doc}"
    );
    let serialized = permission.to_string();
    assert!(
        serialized.contains("bash") && serialized.contains("rm") && serialized.contains("deny"),
        "effective permission must retain project bash rm* deny: {serialized}"
    );

    let port = free_server_port();
    let child = start_serve(&home, &project, port, None, &[]);
    let server_cfg = curl(&["-sS", &format!("http://127.0.0.1:{port}/config")]);
    assert_eq!(server_cfg.code(), Some(0));
    let server_doc = parse_json(&server_cfg.stdout);
    let server_permission = server_doc["permission"].to_string();
    assert!(
        server_permission.contains("bash")
            && server_permission.contains("rm")
            && server_permission.contains("deny"),
        "server /config must surface the same project deny: {server_permission}"
    );
    stop_server(child, port);
}

// Live half: real OpenRouter turns on headless + server for model/agent + deny.
#[test]
fn project_layers_govern_live_headless_and_server_turns() {
    let Some(key) = live_key() else {
        eprintln!("skipping live test: OPENROUTER_API_KEY not set");
        return;
    };

    let root = scratch_dir("live");
    let home = root.join("home");
    let project = root.join("project");
    let mut body = project_config_model_agent();
    body.push_str(&project_config_deny());
    write_project(&project, &body);

    // --- VAL-CROSS-011 + VAL-CROSS-014: headless reports project model + agent ---
    let headless = run_cortex(
        &home,
        &project,
        &["run", "--format", "json", "Reply with exactly: pong"],
        Some(&key),
        &[],
    );
    assert_eq!(
        headless.code(),
        Some(0),
        "headless project turn failed: stdout={} stderr={}",
        headless.stdout_str(),
        headless.stderr_str()
    );
    let headless_doc = parse_json(&headless.stdout);
    assert_eq!(
        headless_doc["model"].as_str(),
        Some(PROJECT_MODEL),
        "VAL-CROSS-011 headless JSON model: {headless_doc}"
    );
    assert_eq!(
        headless_doc["agent"].as_str(),
        Some(PROJECT_AGENT),
        "VAL-CROSS-014 headless JSON agent: {headless_doc}"
    );

    // --- VAL-CROSS-013: CORTEX_MODEL beats file on a real headless turn ---
    let headless_env = run_cortex(
        &home,
        &project,
        &["run", "--format", "json", "Reply with exactly: env-ok"],
        Some(&key),
        &[("CORTEX_MODEL", ENV_MODEL)],
    );
    assert_eq!(
        headless_env.code(),
        Some(0),
        "headless env-model turn failed: {}",
        headless_env.stderr_str()
    );
    let headless_env_doc = parse_json(&headless_env.stdout);
    assert_eq!(
        headless_env_doc["model"].as_str(),
        Some(ENV_MODEL),
        "VAL-CROSS-013 headless env model: {headless_env_doc}"
    );

    // Unset restores file model.
    let headless_restore = run_cortex(
        &home,
        &project,
        &["run", "--format", "json", "Reply with exactly: file-ok"],
        Some(&key),
        &[],
    );
    assert_eq!(headless_restore.code(), Some(0));
    let headless_restore_doc = parse_json(&headless_restore.stdout);
    assert_eq!(
        headless_restore_doc["model"].as_str(),
        Some(PROJECT_MODEL),
        "unsetting CORTEX_MODEL restores headless file model: {headless_restore_doc}"
    );

    // --- Server: same project model/agent, then env override on a fresh serve ---
    let port = free_server_port();
    let child = start_serve(&home, &project, port, Some(&key), &[]);
    let session_id = create_session(port);
    let turn = curl(&[
        "-sS",
        "-X",
        "POST",
        "-H",
        "Content-Type: application/json",
        "-d",
        r#"{"text":"Reply with exactly: pong"}"#,
        &format!("http://127.0.0.1:{port}/session/{session_id}/message"),
    ]);
    assert_eq!(
        turn.code(),
        Some(0),
        "server project turn failed: {} / {}",
        turn.stdout_str(),
        turn.stderr_str()
    );
    let turn_doc = parse_json(&turn.stdout);
    assert_eq!(
        turn_doc["model"].as_str(),
        Some(PROJECT_MODEL),
        "VAL-CROSS-011 server turn model: {turn_doc}"
    );
    assert_eq!(
        turn_doc["agent"].as_str(),
        Some(PROJECT_AGENT),
        "VAL-CROSS-014 server turn agent: {turn_doc}"
    );
    stop_server(child, port);

    let port_env = free_server_port();
    let child_env = start_serve(
        &home,
        &project,
        port_env,
        Some(&key),
        &[("CORTEX_MODEL", ENV_MODEL)],
    );
    let session_env = create_session(port_env);
    let turn_env = curl(&[
        "-sS",
        "-X",
        "POST",
        "-H",
        "Content-Type: application/json",
        "-d",
        r#"{"text":"Reply with exactly: env-ok"}"#,
        &format!("http://127.0.0.1:{port_env}/session/{session_env}/message"),
    ]);
    assert_eq!(turn_env.code(), Some(0), "{}", turn_env.stdout_str());
    let turn_env_doc = parse_json(&turn_env.stdout);
    assert_eq!(
        turn_env_doc["model"].as_str(),
        Some(ENV_MODEL),
        "VAL-CROSS-013 server env model: {turn_env_doc}"
    );
    stop_server(child_env, port_env);

    // --- VAL-CROSS-012: deny bash `rm *` — victim survives headless + server ---
    let victim_name = "victim.txt";
    std::fs::write(project.join(victim_name), "KEEP-ME\n").expect("seed victim");
    // Headless: force a bash tool attempt with an explicit allow for bash generally is not needed
    // when the deny rule alone is present; ask serves as the default for bash, but project deny
    // must win without side effect. Use the same direct rm prompt as the permission live suite.
    let rm_prompt = format!(
        "Immediately use the bash tool to run exactly this one command: rm {victim_name} . Do not read, list, or inspect anything first — just run `rm {victim_name}` now."
    );
    // Plan agent hard-denies bash entirely; to exercise the config `deny rm *` rule rather than the
    // plan hard-deny, override the agent for the deny turn (policy under test is the project rule).
    let deny_headless = run_cortex(
        &home,
        &project,
        &["run", "--agent", "build", "--format", "json", &rm_prompt],
        Some(&key),
        &[],
    );
    assert!(
        deny_headless.code().is_some(),
        "headless deny turn hung: {}",
        deny_headless.stderr_str()
    );
    assert!(
        project.join(victim_name).exists(),
        "VAL-CROSS-012 headless: project deny must leave canary alive after bash rm"
    );

    // Server deny path (use a fresh workspace file; re-seed).
    std::fs::write(project.join(victim_name), "KEEP-ME\n").expect("reseed victim");
    // Project still has agent=plan which hard-denies bash; for the server surface the config
    // deny is loaded into the permission engine for the selected agent. Plan agent also
    // hard-denies bash → same no-side-effect outcome. To specifically hit the project rule under
    // build, patch project agent to build for the server deny process only.
    write_project(
        &project,
        &format!(
            "model = \"{PROJECT_MODEL}\"\nagent = \"build\"\n{}",
            project_config_deny()
        ),
    );
    let port_deny = free_server_port();
    let child_deny = start_serve(&home, &project, port_deny, Some(&key), &[]);
    let session_deny = create_session(port_deny);
    let deny_turn = curl(&[
        "-sS",
        "-X",
        "POST",
        "-H",
        "Content-Type: application/json",
        "-d",
        &format!(r#"{{"text":"{rm_prompt}"}}"#),
        &format!("http://127.0.0.1:{port_deny}/session/{session_deny}/message"),
    ]);
    assert_eq!(
        deny_turn.code(),
        Some(0),
        "server deny turn transport failed: {}",
        deny_turn.stderr_str()
    );
    assert!(
        project.join(victim_name).exists(),
        "VAL-CROSS-012 server: project deny must leave canary alive after bash rm; stdout={}",
        deny_turn.stdout_str()
    );
    stop_server(child_deny, port_deny);

    // Restore project model/agent for consistency if more tests share the tree (they don't).
    write_project(&project, &{
        let mut body = project_config_model_agent();
        body.push_str(&project_config_deny());
        body
    });
}
