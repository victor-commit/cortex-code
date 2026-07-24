//! Live cross-surface integration proof for M7 finishing assertions:
//! VAL-CROSS-009, VAL-CROSS-041, VAL-CROSS-042, VAL-CROSS-043.

use std::io::Read;
use std::net::TcpListener;
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Child, Command, Output, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use serde_json::Value;

const RUN_TIMEOUT: Duration = Duration::from_secs(120);
const SERVER_START_TIMEOUT: Duration = Duration::from_secs(20);

fn live_key() -> Option<String> {
    match std::env::var("OPENROUTER_API_KEY") {
        Ok(key) if !key.trim().is_empty() => Some(key),
        _ => None,
    }
}

fn tuistory_available() -> bool {
    Command::new("tuistory")
        .arg("--version")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

fn scratch_dir(tag: &str) -> PathBuf {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let root = std::env::temp_dir().join(format!(
        "cortex-cross-m7-{tag}-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("home")).expect("create home");
    std::fs::create_dir_all(root.join("project")).expect("create project");
    root
}

struct Sandbox {
    root: PathBuf,
    home: PathBuf,
    project: PathBuf,
    db_path: PathBuf,
}

impl Sandbox {
    fn new(tag: &str) -> Self {
        let root = scratch_dir(tag);
        Self {
            home: root.join("home"),
            project: root.join("project"),
            db_path: root.join("cortex.db"),
            root,
        }
    }

    fn command(&self, key: &str, args: &[&str]) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_cortex"));
        command
            .args(args)
            .current_dir(&self.project)
            .env("HOME", &self.home)
            .env("CORTEX_DB_PATH", &self.db_path)
            .env("OPENROUTER_API_KEY", key)
            .env_remove("CORTEX_NO_NET")
            .env_remove("XDG_DATA_HOME")
            .env_remove("XDG_CONFIG_HOME")
            .env_remove("CORTEX_DATA_DIR")
            .env_remove("CORTEX_MODEL")
            .env_remove("CORTEX_OPENROUTER_BASE_URL")
            .env_remove("RUST_LOG")
            .env_remove("CORTEX_VERBOSE")
            .env_remove("NO_COLOR");
        command
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn run_command(mut command: Command, timeout: Duration) -> Output {
    command.stdin(Stdio::null());
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());

    let mut child = command.spawn().expect("spawn command");
    let mut out = child.stdout.take().expect("stdout");
    let mut err = child.stderr.take().expect("stderr");
    let out_thread = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = out.read_to_end(&mut buf);
        buf
    });
    let err_thread = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = err.read_to_end(&mut buf);
        buf
    });

    let start = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().expect("try_wait") {
            break status;
        }
        if start.elapsed() > timeout {
            let _ = child.kill();
            let _ = child.wait();
            panic!("command did not terminate within {timeout:?}");
        }
        std::thread::sleep(Duration::from_millis(20));
    };

    Output {
        status,
        stdout: out_thread.join().expect("join stdout"),
        stderr: err_thread.join().expect("join stderr"),
    }
}

fn parse_json_output(output: Output, context: &str) -> Value {
    assert_eq!(
        output.status.code(),
        Some(0),
        "{context} must exit 0; stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("strict JSON stdout")
}

fn stdout_text(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr_text(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn new_session_id(sandbox: &Sandbox, key: &str) -> String {
    let doc = parse_json_output(
        run_command(
            sandbox.command(key, &["session", "new", "--format", "json"]),
            RUN_TIMEOUT,
        ),
        "session new",
    );
    doc["id"].as_str().expect("session id").to_owned()
}

fn stats_json(sandbox: &Sandbox, key: &str) -> Value {
    parse_json_output(
        run_command(
            sandbox.command(key, &["stats", "--format", "json"]),
            RUN_TIMEOUT,
        ),
        "stats",
    )
}

fn stats_total_tokens(stats: &Value) -> u64 {
    stats["totals"]["total_tokens"]
        .as_u64()
        .expect("totals.total_tokens")
}

fn stats_session_usage(stats: &Value, session_id: &str) -> (u64, u64, u64, f64) {
    let sessions = stats["sessions"].as_array().expect("stats sessions");
    let row = sessions
        .iter()
        .find(|row| row["session_id"] == session_id)
        .unwrap_or_else(|| panic!("missing stats session row for {session_id}: {stats:?}"));
    (
        row["usage"]["input_tokens"]
            .as_u64()
            .expect("session input"),
        row["usage"]["output_tokens"]
            .as_u64()
            .expect("session output"),
        row["usage"]["total_tokens"]
            .as_u64()
            .expect("session total"),
        row["usage"]["cost"].as_f64().expect("session cost"),
    )
}

fn free_server_port() -> u16 {
    for port in 18765..=18864 {
        if TcpListener::bind(("127.0.0.1", port)).is_ok() {
            return port;
        }
    }
    panic!("no free server port in 18765-18864");
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

fn wait_for_exit(child: &mut Child, timeout: Duration) {
    let start = Instant::now();
    loop {
        if child.try_wait().expect("try_wait").is_some() {
            return;
        }
        if start.elapsed() > timeout {
            let _ = child.kill();
            let _ = child.wait();
            panic!("child process did not terminate within {timeout:?}");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

struct ServerGuard {
    child: Option<Child>,
    port: u16,
}

impl ServerGuard {
    fn start(sandbox: &Sandbox, key: &str) -> Self {
        let port = free_server_port();
        let mut command = sandbox.command(
            key,
            &[
                "serve",
                "--hostname",
                "127.0.0.1",
                "--port",
                &port.to_string(),
            ],
        );
        command.process_group(0);
        command.stdin(Stdio::null());
        command.stdout(Stdio::piped());
        command.stderr(Stdio::piped());
        let child = command.spawn().expect("spawn server");
        let mut guard = Self {
            child: Some(child),
            port,
        };
        guard.wait_for_health();
        guard
    }

    fn port(&self) -> u16 {
        self.port
    }

    fn wait_for_health(&mut self) {
        let start = Instant::now();
        loop {
            let health = run_command(
                {
                    let mut curl = Command::new("curl");
                    curl.args(["-sf", &format!("http://127.0.0.1:{}/health", self.port)]);
                    curl
                },
                Duration::from_secs(10),
            );
            if health.status.code() == Some(0) {
                return;
            }
            assert!(
                start.elapsed() < SERVER_START_TIMEOUT,
                "server failed healthcheck on {}: {}",
                self.port,
                stderr_text(&health)
            );
            std::thread::sleep(Duration::from_millis(100));
        }
    }
}

impl Drop for ServerGuard {
    fn drop(&mut self) {
        let Some(mut child) = self.child.take() else {
            return;
        };
        let pid = listener_pid(self.port).unwrap_or_else(|| child.id());
        let _ = signal_pid("-TERM", pid);
        wait_for_exit(&mut child, Duration::from_secs(10));
        assert!(
            TcpListener::bind(("127.0.0.1", self.port)).is_ok(),
            "server port {} should be released after shutdown",
            self.port
        );
    }
}

fn curl_json(args: &[&str], context: &str) -> Value {
    let mut curl = Command::new("curl");
    curl.args(args);
    let output = run_command(curl, RUN_TIMEOUT);
    assert_eq!(
        output.status.code(),
        Some(0),
        "{context} curl failed: {}",
        stderr_text(&output)
    );
    serde_json::from_slice(&output.stdout).expect("curl strict JSON")
}

fn create_server_session(port: u16) -> String {
    let doc = curl_json(
        &[
            "-sfS",
            "-X",
            "POST",
            "-H",
            "Content-Type: application/json",
            "-d",
            "{}",
            &format!("http://127.0.0.1:{port}/session"),
        ],
        "create server session",
    );
    doc["id"].as_str().expect("server session id").to_owned()
}

fn server_post_message(port: u16, session_id: &str, prompt: &str) -> Value {
    let body = serde_json::json!({ "text": prompt }).to_string();
    curl_json(
        &[
            "-sfS",
            "-X",
            "POST",
            "-H",
            "Content-Type: application/json",
            "-d",
            &body,
            &format!("http://127.0.0.1:{port}/session/{session_id}/message"),
        ],
        "server message",
    )
}

fn server_get_session_messages(port: u16, session_id: &str) -> Value {
    curl_json(
        &[
            "-sfS",
            &format!("http://127.0.0.1:{port}/session/{session_id}/message"),
        ],
        "server session payload",
    )
}

struct TuiSession {
    name: String,
}

impl TuiSession {
    fn launch(sandbox: &Sandbox, key: &str, cortex_args: &[&str]) -> Self {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let name = format!(
            "cortex-cross-m7-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        );
        let command_line = std::iter::once(env!("CARGO_BIN_EXE_cortex").to_owned())
            .chain(cortex_args.iter().map(|value| (*value).to_owned()))
            .collect::<Vec<_>>()
            .join(" ");
        let output = run_command(
            {
                let mut launch = Command::new("tuistory");
                launch
                    .arg("launch")
                    .arg(command_line)
                    .arg("-s")
                    .arg(&name)
                    .arg("--cwd")
                    .arg(&sandbox.project)
                    .arg("--cols")
                    .arg("220")
                    .arg("--rows")
                    .arg("36")
                    .arg("--env")
                    .arg(format!("HOME={}", sandbox.home.display()))
                    .arg("--env")
                    .arg(format!("CORTEX_DB_PATH={}", sandbox.db_path.display()))
                    .arg("--env")
                    .arg(format!("OPENROUTER_API_KEY={key}"))
                    .arg("--env")
                    .arg("CORTEX_NO_NET=")
                    .arg("--env")
                    .arg("CORTEX_OPENROUTER_BASE_URL=")
                    .arg("--env")
                    .arg("CORTEX_AUTH_FILE=")
                    .arg("--env")
                    .arg("RUST_LOG=")
                    .arg("--env")
                    .arg("CORTEX_VERBOSE=")
                    .arg("--env")
                    .arg("NO_COLOR=")
                    .arg("--background")
                    .arg("--timeout")
                    .arg("10000");
                launch
            },
            Duration::from_secs(30),
        );
        assert!(
            output.status.success(),
            "tuistory launch failed:\nstdout={}\nstderr={}",
            stdout_text(&output),
            stderr_text(&output)
        );
        Self { name }
    }

    fn type_text(&self, text: &str) {
        let output = run_command(
            {
                let mut cmd = Command::new("tuistory");
                cmd.args(["-s", &self.name, "type", text]);
                cmd
            },
            Duration::from_secs(20),
        );
        assert!(
            output.status.success(),
            "tuistory type failed:\nstdout={}\nstderr={}",
            stdout_text(&output),
            stderr_text(&output)
        );
    }

    fn press_enter(&self) {
        let output = run_command(
            {
                let mut cmd = Command::new("tuistory");
                cmd.args(["-s", &self.name, "press", "enter"]);
                cmd
            },
            Duration::from_secs(20),
        );
        assert!(
            output.status.success(),
            "tuistory press enter failed:\nstdout={}\nstderr={}",
            stdout_text(&output),
            stderr_text(&output)
        );
    }

    fn wait_for(&self, needle: &str, timeout_ms: u64) {
        let output = run_command(
            {
                let mut cmd = Command::new("tuistory");
                cmd.args([
                    "-s",
                    &self.name,
                    "wait",
                    needle,
                    "--timeout",
                    &timeout_ms.to_string(),
                ]);
                cmd
            },
            Duration::from_secs(90),
        );
        assert!(
            output.status.success(),
            "tuistory wait {needle:?} failed:\nstdout={}\nstderr={}",
            stdout_text(&output),
            stderr_text(&output)
        );
    }

    fn snapshot(&self) -> String {
        let output = run_command(
            {
                let mut cmd = Command::new("tuistory");
                cmd.args(["-s", &self.name, "snapshot", "--trim"]);
                cmd
            },
            Duration::from_secs(30),
        );
        assert!(
            output.status.success(),
            "tuistory snapshot failed:\nstdout={}\nstderr={}",
            stdout_text(&output),
            stderr_text(&output)
        );
        stdout_text(&output)
    }
}

impl Drop for TuiSession {
    fn drop(&mut self) {
        let _ = Command::new("tuistory")
            .args(["-s", &self.name, "close"])
            .output();
    }
}

#[derive(Debug)]
struct FooterUsage {
    input_tokens: u64,
    output_tokens: u64,
    cost: Option<f64>,
}

fn parse_footer_usage(snapshot: &str) -> Option<FooterUsage> {
    let usage_start = snapshot.rfind("usage ")?;
    let usage = &snapshot[usage_start + "usage ".len()..];
    if usage.starts_with('—') {
        return None;
    }

    let up_start = usage.find('↑')?;
    let after_up = &usage[up_start + '↑'.len_utf8()..];
    let input_digits: String = after_up
        .chars()
        .take_while(|ch| ch.is_ascii_digit())
        .collect();
    let input_tokens = input_digits.parse().ok()?;

    let down_start = after_up.find('↓')?;
    let after_down = &after_up[down_start + '↓'.len_utf8()..];
    let output_digits: String = after_down
        .chars()
        .take_while(|ch| ch.is_ascii_digit())
        .collect();
    let output_tokens = output_digits.parse().ok()?;

    let cost = after_down.find('$').and_then(|index| {
        let after_dollar = &after_down[index + 1..];
        let digits: String = after_dollar
            .chars()
            .take_while(|ch| ch.is_ascii_digit() || *ch == '.')
            .collect();
        digits.parse().ok()
    });

    Some(FooterUsage {
        input_tokens,
        output_tokens,
        cost,
    })
}

fn approx_eq(left: f64, right: f64, tolerance: f64) -> bool {
    (left - right).abs() <= tolerance
}

#[test]
fn cross_surface_aggregation_roundtrip_and_search_live() {
    let Some(key) = live_key() else {
        eprintln!("skipping live test: OPENROUTER_API_KEY not set");
        return;
    };
    if !tuistory_available() {
        eprintln!("skipping live test: tuistory is unavailable");
        return;
    }

    let source = Sandbox::new("source");
    let baseline = stats_json(&source, &key);
    assert_eq!(
        stats_total_tokens(&baseline),
        0,
        "fresh store should start with zero stats totals"
    );

    let headless_session = new_session_id(&source, &key);
    let headless_token = format!("M7_HEADLESS_{}", std::process::id());
    let headless_turn = parse_json_output(
        run_command(
            source.command(
                &key,
                &[
                    "run",
                    "--session",
                    &headless_session,
                    "--format",
                    "json",
                    &format!("Reply with exactly this token and nothing else: {headless_token}"),
                ],
            ),
            RUN_TIMEOUT,
        ),
        "headless turn",
    );
    assert!(
        headless_turn["content"]
            .as_str()
            .is_some_and(|content| content.contains(&headless_token)),
        "headless response should contain the marker token: {headless_turn:?}"
    );
    let headless_input = headless_turn["session_usage"]["input_tokens"]
        .as_u64()
        .expect("headless session input tokens");
    let headless_output = headless_turn["session_usage"]["output_tokens"]
        .as_u64()
        .expect("headless session output tokens");
    let headless_total = headless_turn["session_usage"]["total_tokens"]
        .as_u64()
        .expect("headless session total tokens");
    let headless_cost = headless_turn["session_usage"]["cost"]
        .as_f64()
        .expect("headless session cost");
    assert!(
        headless_total > 0,
        "headless session usage must be non-zero"
    );

    let stats_after_headless = stats_json(&source, &key);
    assert!(
        stats_total_tokens(&stats_after_headless) > stats_total_tokens(&baseline),
        "stats totals should increase after headless turn"
    );

    let server = ServerGuard::start(&source, &key);
    let headless_server_payload = server_get_session_messages(server.port(), &headless_session);
    assert_eq!(
        headless_server_payload["usage"]["input_tokens"].as_u64(),
        Some(headless_input),
        "server session payload input tokens should match headless persisted usage"
    );
    assert_eq!(
        headless_server_payload["usage"]["output_tokens"].as_u64(),
        Some(headless_output),
        "server session payload output tokens should match headless persisted usage"
    );
    assert_eq!(
        headless_server_payload["usage"]["total_tokens"].as_u64(),
        Some(headless_total),
        "server session payload total tokens should match headless persisted usage"
    );
    let server_cost = headless_server_payload["usage"]["cost"]
        .as_f64()
        .expect("server usage cost");
    assert!(
        approx_eq(server_cost, headless_cost, 1e-9),
        "server session payload cost should match headless cost (server={server_cost}, headless={headless_cost})"
    );

    {
        let tui = TuiSession::launch(&source, &key, &["tui", "--session", &headless_session]);
        tui.wait_for(&headless_session, 20_000);
        let snapshot = tui.snapshot();
        let footer = parse_footer_usage(&snapshot)
            .unwrap_or_else(|| panic!("expected non-empty footer usage in snapshot:\n{snapshot}"));
        assert_eq!(
            footer.input_tokens, headless_input,
            "TUI footer input tokens should match persisted headless usage\nsnapshot:\n{snapshot}"
        );
        assert_eq!(
            footer.output_tokens, headless_output,
            "TUI footer output tokens should match persisted headless usage\nsnapshot:\n{snapshot}"
        );
        let footer_cost = footer
            .cost
            .unwrap_or_else(|| panic!("expected footer cost in snapshot:\n{snapshot}"));
        let expected_footer_cost = (headless_cost * 10_000.0).round() / 10_000.0;
        assert!(
            approx_eq(footer_cost, expected_footer_cost, 0.0001),
            "TUI footer cost should match rounded persisted cost (footer={footer_cost}, expected={expected_footer_cost})\nsnapshot:\n{snapshot}"
        );
    }

    let tui_session = new_session_id(&source, &key);
    let tui_token = format!("M7_TUI_{}", std::process::id());
    {
        let tui = TuiSession::launch(&source, &key, &["tui", "--session", &tui_session]);
        tui.type_text(&format!(
            "Reply with exactly this token and nothing else: {tui_token}"
        ));
        tui.press_enter();
        let started = Instant::now();
        loop {
            let snapshot = tui.snapshot();
            let seen_twice = snapshot.matches(&tui_token).count() >= 2;
            let usage_ready = parse_footer_usage(&snapshot).is_some();
            if seen_twice && usage_ready {
                break;
            }
            assert!(
                started.elapsed() < Duration::from_secs(90),
                "timed out waiting for the TUI turn to settle:\n{snapshot}"
            );
            std::thread::sleep(Duration::from_millis(200));
        }
    }
    let stats_after_tui = stats_json(&source, &key);
    assert!(
        stats_total_tokens(&stats_after_tui) > stats_total_tokens(&stats_after_headless),
        "stats totals should increase after a TUI turn"
    );

    let server_session = create_server_session(server.port());
    let server_token = format!("M7_SERVER_{}", std::process::id());
    let server_turn = server_post_message(
        server.port(),
        &server_session,
        &format!("Reply with exactly this token and nothing else: {server_token}"),
    );
    assert!(
        server_turn["content"]
            .as_str()
            .is_some_and(|content| content.contains(&server_token)),
        "server response should contain the marker token: {server_turn:?}"
    );
    assert!(
        server_turn["usage"]["total_tokens"]
            .as_u64()
            .is_some_and(|value| value > 0),
        "server turn usage should be non-zero: {server_turn:?}"
    );

    let stats_after_server = stats_json(&source, &key);
    assert!(
        stats_total_tokens(&stats_after_server) > stats_total_tokens(&stats_after_tui),
        "stats totals should increase after a server turn"
    );

    let (h_in, h_out, h_total, _) = stats_session_usage(&stats_after_server, &headless_session);
    let (t_in, t_out, t_total, _) = stats_session_usage(&stats_after_server, &tui_session);
    let (s_in, s_out, s_total, _) = stats_session_usage(&stats_after_server, &server_session);
    assert!(
        h_total > 0 && t_total > 0 && s_total > 0,
        "each surface must contribute non-zero usage"
    );
    assert!(h_in > 0 && h_out > 0, "headless tokens should be non-zero");
    assert!(t_in > 0 && t_out > 0, "TUI tokens should be non-zero");
    assert!(s_in > 0 && s_out > 0, "server tokens should be non-zero");
    assert_eq!(
        stats_total_tokens(&stats_after_server),
        h_total + t_total + s_total,
        "aggregate stats total should include headless + TUI + server contributions"
    );

    let search_tui = run_command(source.command(&key, &["search", &tui_token]), RUN_TIMEOUT);
    assert_eq!(
        search_tui.status.code(),
        Some(0),
        "search for TUI token should succeed: {}",
        stderr_text(&search_tui)
    );
    let search_tui_stdout = stdout_text(&search_tui);
    assert!(
        search_tui_stdout.contains(&format!("session:{tui_session}:")),
        "search should include a session hit for the TUI-created token:\n{search_tui_stdout}"
    );
    assert!(
        search_tui_stdout.contains(&tui_token),
        "search output should include the TUI token:\n{search_tui_stdout}"
    );

    let search_server = run_command(
        source.command(&key, &["search", &server_token]),
        RUN_TIMEOUT,
    );
    assert_eq!(
        search_server.status.code(),
        Some(0),
        "search for server token should succeed: {}",
        stderr_text(&search_server)
    );
    let search_server_stdout = stdout_text(&search_server);
    assert!(
        search_server_stdout.contains(&format!("session:{server_session}:")),
        "search should include a session hit for the server-created token:\n{search_server_stdout}"
    );
    assert!(
        search_server_stdout.contains(&server_token),
        "search output should include the server token:\n{search_server_stdout}"
    );

    let export = run_command(
        source.command(&key, &["export", &tui_session]),
        Duration::from_secs(30),
    );
    assert_eq!(
        export.status.code(),
        Some(0),
        "export should succeed: {}",
        stderr_text(&export)
    );
    let artifact_path = source.root.join("session-export.json");
    std::fs::write(&artifact_path, &export.stdout).expect("write export artifact");

    let target = Sandbox::new("target");
    let import = run_command(
        target.command(
            &key,
            &["import", artifact_path.to_str().expect("artifact path")],
        ),
        Duration::from_secs(30),
    );
    assert_eq!(
        import.status.code(),
        Some(0),
        "import should succeed: {}",
        stderr_text(&import)
    );

    let listed = parse_json_output(
        run_command(
            target.command(&key, &["session", "list", "--format", "json"]),
            RUN_TIMEOUT,
        ),
        "target session list",
    );
    let listed_ids: Vec<&str> = listed["sessions"]
        .as_array()
        .expect("sessions array")
        .iter()
        .map(|session| session["id"].as_str().expect("session id"))
        .collect();
    assert!(
        listed_ids.contains(&tui_session.as_str()),
        "imported session should appear in `cortex session list`: {listed:?}"
    );

    {
        let tui = TuiSession::launch(&target, &key, &["tui"]);
        tui.type_text("/sessions");
        tui.press_enter();
        tui.wait_for(&tui_session, 20_000);
        let snapshot = tui.snapshot();
        assert!(
            snapshot.contains(&tui_session),
            "imported session should appear in the TUI picker:\n{snapshot}"
        );
    }

    {
        let tui = TuiSession::launch(&target, &key, &["tui", "--session", &tui_session]);
        tui.wait_for(&tui_token, 20_000);
        let snapshot = tui.snapshot();
        assert!(
            snapshot.contains(&tui_token),
            "attached imported session should show prior transcript content:\n{snapshot}"
        );
    }

    let continued = parse_json_output(
        run_command(
            target.command(
                &key,
                &[
                    "run",
                    "--session",
                    &tui_session,
                    "--format",
                    "json",
                    "Reply with exactly CONTINUED_M7_TOKEN",
                ],
            ),
            RUN_TIMEOUT,
        ),
        "continue imported session",
    );
    assert!(
        continued["content"]
            .as_str()
            .is_some_and(|content| content.contains("CONTINUED_M7_TOKEN")),
        "imported session should be resumable via headless run: {continued:?}"
    );

    let shown = parse_json_output(
        run_command(
            target.command(&key, &["session", "show", &tui_session, "--format", "json"]),
            RUN_TIMEOUT,
        ),
        "target session show",
    );
    assert!(
        shown["messages"]
            .as_array()
            .expect("messages array")
            .iter()
            .any(|message| message["content"] == tui_token),
        "imported session should preserve prior token content: {shown:?}"
    );
}
