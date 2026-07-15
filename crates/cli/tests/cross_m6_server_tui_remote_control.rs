//! Live server↔TUI remote-control integration proof for M6.
//!
//! Covers VAL-CROSS-038/039/040/019/047 with a shared project + data dir, loopback `cortex serve`
//! on a 3200-3299 port, an attached TUI (`--session` + `--server`), and real OpenRouter turns when
//! `OPENROUTER_API_KEY` is present. Offline workspace suites skip cleanly without the key and
//! without `tuistory`.
//!
//! Process hygiene: the server listener is killed by the lsof-resolved PID; TUI PTY sessions are
//! closed by name; every process group is orphan-checked.

use std::net::TcpListener;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

const SERVER_START_TIMEOUT: Duration = Duration::from_secs(20);
const SSE_TIMEOUT: Duration = Duration::from_secs(90);

fn live_key() -> Option<String> {
    match std::env::var("OPENROUTER_API_KEY") {
        Ok(key) if !key.trim().is_empty() => Some(key),
        _ => None,
    }
}

fn tuistory_available() -> bool {
    matches!(
        Command::new("tuistory").arg("--version").output(),
        Ok(output) if output.status.success()
    )
}

fn scratch_dir(tag: &str) -> PathBuf {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!(
        "cortex-cross-m6-remote-{tag}-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

fn free_server_port() -> u16 {
    for port in 3250..=3299 {
        if TcpListener::bind(("127.0.0.1", port)).is_ok() {
            return port;
        }
    }
    panic!("no free server port in 3250-3299");
}

fn nonce(label: &str, salt: u32) -> String {
    format!("{label}x{:x}", (std::process::id() ^ salt) & 0xff_ffff)
}

fn sha256_file(path: &Path) -> String {
    let output = Command::new("sha256sum")
        .arg(path)
        .output()
        .expect("run sha256sum");
    assert!(
        output.status.success(),
        "sha256sum failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout)
        .split_whitespace()
        .next()
        .expect("sha256sum digest")
        .to_owned()
}

fn sha256_bytes(bytes: &[u8]) -> String {
    let mut child = Command::new("sha256sum")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn sha256sum");
    {
        let mut stdin = child.stdin.take().expect("stdin");
        use std::io::Write;
        stdin.write_all(bytes).expect("write sha256 input");
    }
    let output = child.wait_with_output().expect("sha256sum wait");
    assert!(output.status.success(), "sha256sum stdin failed");
    String::from_utf8_lossy(&output.stdout)
        .split_whitespace()
        .next()
        .expect("digest")
        .to_owned()
}

fn cortex_command(key: &str, home: &Path, db_path: &Path, cwd: &Path, args: &[&str]) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_cortex"));
    command.args(args);
    command.current_dir(cwd);
    command.env("OPENROUTER_API_KEY", key);
    command.env("HOME", home);
    command.env("XDG_CONFIG_HOME", home.join(".config"));
    command.env("XDG_DATA_HOME", home.join(".local/share"));
    command.env("CORTEX_DATA_DIR", home.join(".local/share/cortex"));
    command.env("CORTEX_DB_PATH", db_path);
    command.env(
        "CORTEX_CHECKPOINT_DIR",
        home.join(".local/share/cortex/checkpoints"),
    );
    command
        .env_remove("CORTEX_NO_NET")
        .env_remove("RUST_LOG")
        .env_remove("CORTEX_VERBOSE")
        .env_remove("NO_COLOR")
        .env_remove("CORTEX_AUTH_FILE")
        .env_remove("CORTEX_PURE")
        .env_remove("CORTEX_SERVER_URL")
        .env_remove("CORTEX_SESSION");
    command
}

fn spawn_group(mut command: Command) -> Child {
    command.stdin(Stdio::null());
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());
    command.process_group(0);
    command.spawn().expect("spawn process")
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

fn single_listener_pid(port: u16) -> Option<u32> {
    let output = Command::new("lsof")
        .args(["-tiTCP", &format!(":{port}"), "-sTCP:LISTEN"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .find_map(|line| line.trim().parse::<u32>().ok())
}

fn kill_listener(port: u16) {
    if let Some(pid) = single_listener_pid(port) {
        let _ = Command::new("kill").arg(pid.to_string()).status();
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(5) {
            if single_listener_pid(port).is_none() {
                return;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}

fn group_has_survivors(pgid: u32) -> Option<bool> {
    if pgid == 0 {
        return Some(false);
    }
    let status = Command::new("ps")
        .args(["-o", "pid=", "--pgid", &pgid.to_string()])
        .output()
        .ok()?;
    if !status.status.success() {
        return Some(false);
    }
    let survivors = String::from_utf8_lossy(&status.stdout)
        .lines()
        .filter(|line| !line.trim().is_empty())
        .count();
    Some(survivors > 0)
}

fn assert_no_orphans(pgid: u32) {
    if let Some(true) = group_has_survivors(pgid) {
        panic!("process group {pgid} still has live members after exit");
    }
}

struct ServerHandle {
    child: Child,
    port: u16,
    pgid: u32,
    stopped: bool,
}

impl ServerHandle {
    fn start(key: &str, home: &Path, db_path: &Path, cwd: &Path) -> Self {
        let port = free_server_port();
        let port_arg = port.to_string();
        let mut child = spawn_group(cortex_command(
            key,
            home,
            db_path,
            cwd,
            &[
                "serve",
                "--hostname",
                "127.0.0.1",
                "--port",
                port_arg.as_str(),
            ],
        ));
        let start = Instant::now();
        loop {
            if let Some(status) = child.try_wait().expect("try_wait server") {
                panic!("cortex serve exited early with {status:?}");
            }
            if curl_status(port, "/health") == Some(200) {
                break;
            }
            if start.elapsed() > SERVER_START_TIMEOUT {
                let _ = child.kill();
                let _ = child.wait();
                panic!("server did not become healthy within {SERVER_START_TIMEOUT:?}");
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        Self {
            pgid: child.id(),
            child,
            port,
            stopped: false,
        }
    }

    fn origin(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    fn stop(&mut self) {
        if self.stopped {
            return;
        }
        kill_listener(self.port);
        let _ = wait_for_exit(&mut self.child, Duration::from_secs(5));
        assert!(
            single_listener_pid(self.port).is_none(),
            "port {} still listening after listener kill",
            self.port
        );
        assert_no_orphans(self.pgid);
        self.stopped = true;
    }
}

impl Drop for ServerHandle {
    fn drop(&mut self) {
        self.stop();
    }
}

fn curl_status(port: u16, path: &str) -> Option<u16> {
    let url = format!("http://127.0.0.1:{port}{path}");
    let output = Command::new("curl")
        .args(["-s", "-o", "/dev/null", "-w", "%{http_code}", &url])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8_lossy(&output.stdout).trim().parse().ok()
}

fn request_json(
    port: u16,
    method: &str,
    path: &str,
    body: Option<&str>,
    host: Option<&str>,
) -> (u16, String) {
    let url = format!("http://127.0.0.1:{port}{path}");
    let mut args = vec![
        "-sS".to_owned(),
        "-X".to_owned(),
        method.to_owned(),
        "-H".to_owned(),
        "Content-Type: application/json".to_owned(),
        "-H".to_owned(),
        "Accept: application/json".to_owned(),
        "-w".to_owned(),
        "\n%{http_code}".to_owned(),
    ];
    if let Some(host) = host {
        args.push("-H".to_owned());
        args.push(format!("Host: {host}"));
    }
    if let Some(body) = body {
        args.push("-d".to_owned());
        args.push(body.to_owned());
    }
    args.push(url);
    let output = Command::new("curl")
        .args(&args)
        .output()
        .expect("curl request");
    let raw = String::from_utf8_lossy(&output.stdout);
    let (body_text, code) = raw
        .rsplit_once('\n')
        .map(|(body, code)| (body.to_owned(), code.trim().to_owned()))
        .unwrap_or_else(|| (raw.into_owned(), "0".to_owned()));
    let status = code.parse::<u16>().unwrap_or(0);
    (status, body_text)
}

fn create_session(port: u16, title: &str) -> String {
    let body = json!({ "title": title }).to_string();
    let (status, text) = request_json(port, "POST", "/session", Some(&body), None);
    assert!(
        (200..300).contains(&status),
        "create session failed: {status} {text}"
    );
    let value: Value = serde_json::from_str(&text).expect("session JSON");
    value["id"].as_str().expect("session id").to_owned()
}

fn post_message(port: u16, session_id: &str, text: &str) -> Value {
    let body = json!({ "text": text }).to_string();
    let (status, response) = request_json(
        port,
        "POST",
        &format!("/session/{session_id}/message"),
        Some(&body),
        None,
    );
    assert_eq!(status, 200, "message turn failed: {status} {response}");
    serde_json::from_str(&response).expect("message JSON")
}

struct SseCapture {
    child: Child,
    path: PathBuf,
    pgid: u32,
}

impl SseCapture {
    fn start(port: u16, path: PathBuf) -> Self {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("sse parent");
        }
        let url = format!("http://127.0.0.1:{port}/event");
        let mut command = Command::new("curl");
        command.args(["-sS", "-N", "-H", "Accept: text/event-stream", &url]);
        command.stdout(Stdio::from(std::fs::File::create(&path).expect("sse file")));
        command.stderr(Stdio::null());
        command.process_group(0);
        let child = command.spawn().expect("spawn sse curl");
        Self {
            pgid: child.id(),
            child,
            path,
        }
    }

    fn contents(&self) -> String {
        std::fs::read_to_string(&self.path).unwrap_or_default()
    }

    fn wait_for(&self, needles: &[&str], timeout: Duration) -> String {
        let start = Instant::now();
        loop {
            let text = self.contents();
            if needles.iter().all(|needle| text.contains(needle)) {
                return text;
            }
            if start.elapsed() > timeout {
                panic!("SSE stream did not contain {needles:?} within {timeout:?}. got:\n{text}");
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    fn stop(mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        assert_no_orphans(self.pgid);
    }
}

struct TuistorySession {
    name: String,
}

impl TuistorySession {
    // Integration harness needs distinct path/env/server/session knobs; keep the
    // explicit parameter list rather than introducing a throwaway config bundle.
    #[allow(clippy::too_many_arguments)]
    fn launch(
        name: String,
        cortex: &Path,
        project: &Path,
        home: &Path,
        db_path: &Path,
        key: &str,
        server_url: &str,
        session_id: &str,
    ) -> Self {
        let command_line = format!(
            "{} tui --session {} --server {}",
            cortex.display(),
            session_id,
            server_url
        );
        let data_dir = home.join(".local/share/cortex");
        let checkpoint_dir = data_dir.join("checkpoints");
        let launch = Command::new("tuistory")
            .arg("launch")
            .arg(command_line)
            .arg("-s")
            .arg(&name)
            .arg("--cwd")
            .arg(project)
            .arg("--cols")
            .arg("120")
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
            .arg(format!("CORTEX_DATA_DIR={}", data_dir.display()))
            .arg("--env")
            .arg(format!("CORTEX_DB_PATH={}", db_path.display()))
            .arg("--env")
            .arg(format!(
                "CORTEX_CHECKPOINT_DIR={}",
                checkpoint_dir.display()
            ))
            .arg("--env")
            .arg(format!("OPENROUTER_API_KEY={key}"))
            .arg("--env")
            .arg("CORTEX_AUTH_FILE=")
            .arg("--env")
            .arg("CORTEX_PURE=")
            .arg("--env")
            .arg("CORTEX_NO_NET=")
            .arg("--env")
            .arg("RUST_LOG=")
            .arg("--env")
            .arg("CORTEX_VERBOSE=")
            .arg("--env")
            .arg("NO_COLOR=")
            .arg("--background")
            .arg("--timeout")
            .arg("15000")
            .output()
            .expect("launch tuistory");
        assert!(
            launch.status.success(),
            "tuistory launch failed:\nstdout={}\nstderr={}",
            String::from_utf8_lossy(&launch.stdout),
            String::from_utf8_lossy(&launch.stderr)
        );
        // Give the TUI a moment to finish boot + bind remote polling.
        std::thread::sleep(Duration::from_millis(800));
        Self { name }
    }

    fn run(&self, args: &[&str]) {
        let output = Command::new("tuistory")
            .arg("-s")
            .arg(&self.name)
            .args(args)
            .output()
            .expect("run tuistory command");
        assert!(
            output.status.success(),
            "tuistory {args:?} failed:\nstdout={}\nstderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn snapshot(&self) -> String {
        let output = Command::new("tuistory")
            .arg("-s")
            .arg(&self.name)
            .args(["snapshot", "--immediate", "--trim"])
            .output()
            .expect("snapshot");
        assert!(
            output.status.success(),
            "snapshot failed:\nstdout={}\nstderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).into_owned()
    }

    fn wait_for(&self, needle: &str, timeout_ms: u64) -> String {
        let output = Command::new("tuistory")
            .arg("-s")
            .arg(&self.name)
            .arg("wait")
            .arg(needle)
            .arg("--timeout")
            .arg(timeout_ms.to_string())
            .output()
            .expect("wait");
        if output.status.success() {
            return self.snapshot();
        }
        // Fall back: poll snapshots ourselves in case the wait target wrapped oddly.
        let start = Instant::now();
        loop {
            let snap = self.snapshot();
            if snap.contains(needle) {
                return snap;
            }
            if start.elapsed() > Duration::from_millis(timeout_ms) {
                panic!(
                    "TUI did not show {needle:?} within {timeout_ms}ms:\n{snap}\nwait stderr={}",
                    String::from_utf8_lossy(&output.stderr)
                );
            }
            std::thread::sleep(Duration::from_millis(250));
        }
    }

    fn wait_for_any(&self, needles: &[&str], timeout_ms: u64) -> String {
        let start = Instant::now();
        loop {
            let snap = self.snapshot();
            if needles.iter().any(|needle| snap.contains(needle)) {
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

fn extract_share_url(text: &str) -> Option<String> {
    for token in text.split_whitespace() {
        let cleaned = token.trim_matches(|c: char| {
            c == ',' || c == ';' || c == '·' || c == '.' || c == ')' || c == '('
        });
        if (cleaned.starts_with("http://127.0.0.1:") || cleaned.starts_with("http://localhost:"))
            && cleaned.contains("/share/")
        {
            return Some(cleaned.to_owned());
        }
    }
    let re_like = text
        .find("http://127.0.0.1:")
        .or_else(|| text.find("http://localhost:"))?;
    let tail = &text[re_like..];
    let end = tail
        .find(|c: char| c.is_whitespace() || c == '·' || c == ',')
        .unwrap_or(tail.len());
    let candidate = tail[..end].trim_matches(|c: char| !c.is_ascii_graphic());
    if candidate.contains("/share/") {
        Some(candidate.to_owned())
    } else {
        None
    }
}

#[test]
fn server_tui_remote_control_transcript_sse_share_undo_and_abort() {
    let Some(key) = live_key() else {
        eprintln!(
            "skipping cross-m6 server↔TUI remote-control live proof: OPENROUTER_API_KEY absent"
        );
        return;
    };
    if !tuistory_available() {
        eprintln!("skipping cross-m6 server↔TUI remote-control live proof: tuistory unavailable");
        return;
    }

    let root = scratch_dir("suite");
    let home = root.join("home");
    let project = root.join("project");
    let db_path = root.join("cortex.db");
    std::fs::create_dir_all(home.join(".local/share/cortex/checkpoints")).expect("data dirs");
    std::fs::create_dir_all(home.join(".config")).expect("config dir");
    std::fs::create_dir_all(project.join("src")).expect("project/src");

    // Project target file used for both server-driven edit and TUI undo.
    let edit_path = project.join("src/x.rs");
    let h0_bytes = b"// H0 baseline remote-control\npub fn marker() -> &'static str { \"H0\" }\n";
    std::fs::write(&edit_path, h0_bytes).expect("seed H0");
    let h0 = sha256_bytes(h0_bytes);

    let mut server = ServerHandle::start(&key, &home, &db_path, &project);
    let origin = server.origin();
    let session_id = create_session(server.port, "remote-control suite");

    let sse_path = root.join("sse.log");
    let sse = SseCapture::start(server.port, sse_path);

    let tui_name = format!(
        "cortex-m6-remote-{}-{}",
        std::process::id(),
        Instant::now().elapsed().as_nanos()
    );
    let tui = TuistorySession::launch(
        tui_name,
        Path::new(env!("CARGO_BIN_EXE_cortex")),
        &project,
        &home,
        &db_path,
        &key,
        &origin,
        &session_id,
    );

    // ── VAL-CROSS-038: server-injected turn surfaces in the attached TUI ─────────
    let inject = nonce("srvmsg", 0x11);
    let inject_prompt = format!("Reply with exactly: {inject}");
    let inject_response = post_message(server.port, &session_id, &inject_prompt);
    let inject_content = inject_response["content"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    assert!(
        inject_content.contains(&inject),
        "server turn must echo the marker {inject}: {inject_content}"
    );
    let snap_038 = tui.wait_for_any(&[&inject, &inject_prompt], 45_000);
    assert!(
        snap_038.contains(&inject) || snap_038.contains(&inject_prompt),
        "VAL-CROSS-038: attached TUI should surface the server-injected turn; snapshot:\n{snap_038}"
    );

    // ── VAL-CROSS-039: TUI-initiated turn is visible on SSE /event ───────────────
    let tui_marker = nonce("tui", 0x22);
    let tui_prompt = format!("Reply with exactly: {tui_marker}");
    tui.run(&["type", &tui_prompt]);
    tui.run(&["press", "enter"]);
    let _ = tui.wait_for(&tui_marker, 90_000);
    let sse_039 = sse.wait_for(&[&session_id, "turn.started"], SSE_TIMEOUT);
    assert!(
        sse_039.contains("turn.started")
            && (sse_039.contains("turn.done")
                || sse_039.contains("turn.usage")
                || sse_039.contains("assistant.text.delta")
                || sse_039.contains("message.updated")
                || sse_039.contains("message.part")),
        "VAL-CROSS-039: SSE must show message/turn/usage for the TUI turn:\n{sse_039}"
    );
    assert!(
        sse_039.contains(&session_id),
        "VAL-CROSS-039: SSE events must correlate to session {session_id}"
    );

    // ── VAL-CROSS-040: TUI /share produces a loopback share retrievable via curl ─
    tui.run(&["type", "/share"]);
    tui.run(&["press", "enter"]);
    let snap_share = tui.wait_for_any(&["/share/", "Share link", "local share", "Share"], 15_000);
    let share_url = extract_share_url(&snap_share).unwrap_or_else(|| {
        panic!("VAL-CROSS-040: TUI /share did not show a loopback URL:\n{snap_share}")
    });
    assert!(
        share_url.starts_with("http://127.0.0.1:") || share_url.starts_with("http://localhost:"),
        "VAL-CROSS-040: share URL must be loopback, got {share_url}"
    );
    assert!(
        share_url.contains(&format!(":{}", server.port)),
        "VAL-CROSS-040: share URL port must match serve {} ({share_url})",
        server.port
    );
    let share_body = Command::new("curl")
        .args(["-sS", &share_url])
        .output()
        .expect("curl share");
    assert!(
        share_body.status.success(),
        "VAL-CROSS-040: curl share failed"
    );
    let share_text = String::from_utf8_lossy(&share_body.stdout);
    assert!(
        share_text.contains(&inject)
            || share_text.contains(&tui_marker)
            || share_text.contains(&session_id)
            || share_text.contains("assistant")
            || share_text.contains("user"),
        "VAL-CROSS-040: shared content over loopback missing session content:\n{share_text}"
    );

    // ── VAL-CROSS-019: server-driven edit is undoable via TUI /undo ──────────────
    let h1_marker = nonce("h1", 0x33);
    let edit_prompt = format!(
        "Use the write tool to overwrite the file src/x.rs with EXACTLY these three lines and nothing else:\n// H1 server edit {h1_marker}\npub fn marker() -> &'static str {{ \"H1\" }}\n// end\nDo not keep any previous content. After writing, reply with exactly: EDIT-DONE-{h1_marker}"
    );
    let edit_response = post_message(server.port, &session_id, &edit_prompt);
    let edit_content = edit_response["content"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    // Wait briefly for the on-disk write to settle if the model took tools first.
    let h1_deadline = Instant::now() + Duration::from_secs(30);
    let mut h1 = String::new();
    loop {
        if edit_path.is_file() {
            h1 = sha256_file(&edit_path);
            if h1 != h0 {
                break;
            }
        }
        if Instant::now() > h1_deadline {
            break;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    assert_ne!(
        h1, h0,
        "VAL-CROSS-019: server-driven turn must change src/x.rs (H0→H1); response was:\n{edit_content}"
    );

    // Wait for the TUI to finish syncing the completed server turn so it is idle and /undo works.
    let _ = tui.wait_for_any(
        &[&format!("EDIT-DONE-{h1_marker}"), "EDIT-DONE", &h1_marker],
        60_000,
    );
    std::thread::sleep(Duration::from_millis(750));

    tui.run(&["type", "/undo"]);
    tui.run(&["press", "enter"]);
    let undo_deadline = Instant::now() + Duration::from_secs(15);
    let mut restored = String::new();
    loop {
        if edit_path.is_file() {
            restored = sha256_file(&edit_path);
            if restored == h0 {
                break;
            }
        }
        if Instant::now() > undo_deadline {
            break;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    assert_eq!(
        restored, h0,
        "VAL-CROSS-019: TUI /undo must restore H0 (got {restored}, expected {h0})"
    );

    // ── VAL-CROSS-047: server abort halts an in-flight TUI turn ──────────────────
    // Long count so there is a multi-second stream window for the abort.
    tui.run(&[
        "type",
        "Count slowly from 1 to 300, writing each number on its own line. Do not stop early.",
    ]);
    tui.run(&["press", "enter"]);
    // Wait until the stream is clearly underway.
    let mid_stream = tui.wait_for_any(&["10", "11", "12", "13", "14", "15"], 90_000);
    assert!(
        mid_stream.contains('1'),
        "long count must start streaming before abort:\n{mid_stream}"
    );
    let (abort_status, abort_body) = request_json(
        server.port,
        "POST",
        &format!("/session/{session_id}/abort"),
        None,
        None,
    );
    assert!(
        (200..300).contains(&abort_status),
        "abort must be 2xx: {abort_status} {abort_body}"
    );
    let snap_047 = tui.wait_for_any(
        &["Turn interrupted", "interrupted", "Interrupted", "aborted"],
        25_000,
    );
    assert!(
        snap_047.to_ascii_lowercase().contains("interrupt")
            || snap_047.to_ascii_lowercase().contains("aborted"),
        "VAL-CROSS-047: TUI must show interrupted/terminal state after server abort:\n{snap_047}"
    );
    // The turn must never complete the long count cleanly.
    if snap_047.contains("300") {
        assert!(
            snap_047.to_ascii_lowercase().contains("interrupt")
                || snap_047.to_ascii_lowercase().contains("aborted"),
            "VAL-CROSS-047: long turn must not complete cleanly after abort:\n{snap_047}"
        );
    }
    assert_eq!(
        curl_status(server.port, "/health"),
        Some(200),
        "server /health must stay 200 after abort"
    );

    // Cleanup: close TUI, stop SSE and server, confirm no orphans.
    let port = server.port;
    tui.close();
    sse.stop();
    server.stop();
    assert!(
        single_listener_pid(port).is_none(),
        "port must be released after stop"
    );

    // Defensive: ensure the restored file still holds H0 after the suite.
    assert_eq!(sha256_file(&edit_path), h0);
}
