//! Black-box behavior of the Wave 9 `cortex serve` banner + verbose access log.
//!
//! These tests spawn the REAL `cortex serve` binary and assert its human/stderr surface, proving:
//! - **stdout stays clean** — the banner/log are stderr-only, so machine consumers reading stdout
//!   see nothing (VAL-CLI-015).
//! - **no ANSI when stderr is not a TTY** — the child's stderr is a pipe here, so the styler
//!   auto-disables and emits zero escape bytes (VAL-CLI-015).
//! - **the access log is `-v`-gated** — plain `serve` prints only the listening banner with zero
//!   per-request rows; `serve -v` adds one aligned row per probed request (VAL-CLI-013 / 037).
//! - **HTTP behavior is unchanged** — `/health`, `/openapi.json`, and `POST /session` respond
//!   identically whether or not the access log is on (VAL-CLI-016).
//!
//! Reachability: fully offline (`CORTEX_NO_NET=1`, throwaway DB). No OpenRouter key is used; the
//! probed routes are local persistence/health reads.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant};

// Serialize serve spawns in this file so concurrent processes do not contend for the loopback
// port pool; the nextest `network` group additionally caps cross-file concurrency.
static SERVE_BANNER_LOCK: Mutex<()> = Mutex::new(());
static SCRATCH_COUNTER: AtomicU64 = AtomicU64::new(0);
const WAIT_TIMEOUT: Duration = Duration::from_secs(30);

#[test]
fn plain_serve_prints_only_the_banner_with_no_access_rows() {
    let _guard = SERVE_BANNER_LOCK.lock().expect("serve banner lock");
    let port = free_port();
    let scratch = Scratch::new("serve-banner-plain");
    let mut server = ServerProcess::start(port, &scratch.db_path(), &[]);

    // Probe a couple of routes; plain serve must NOT log them.
    let health = request_http(port, "GET", "/health", None).expect("health request");
    assert_eq!(health.status, 200);
    let created = request_http(port, "POST", "/session", Some("{}")).expect("create session");
    assert_eq!(created.status, 201);

    let (stdout, stderr) = server.stop_and_capture();

    assert!(
        stdout.is_empty(),
        "plain serve must keep stdout clean, got: {stdout:?}"
    );
    assert!(
        !stderr.contains('\x1b'),
        "piped (non-TTY) stderr must carry no ANSI escape, got: {stderr:?}"
    );
    let banner = format!("cortex serve listening on http://127.0.0.1:{port}");
    assert!(
        stderr.contains(&banner),
        "plain serve must print the listening banner, got:\n{stderr}"
    );
    // No per-request access-log rows: neither a method token nor a probed path appears as a row.
    for row_marker in ["GET ", "POST ", "/health", "/session"] {
        assert!(
            !stderr.contains(row_marker),
            "plain serve must not emit access-log rows (found {row_marker:?}) in:\n{stderr}"
        );
    }
    // And no routes-hint footer either (that is part of the verbose surface).
    assert!(
        !stderr.contains("routes:"),
        "plain serve must not emit the routes hint, got:\n{stderr}"
    );
}

#[test]
fn verbose_serve_adds_banner_routes_hint_and_aligned_access_rows() {
    let _guard = SERVE_BANNER_LOCK.lock().expect("serve banner lock");
    let port = free_port();
    let scratch = Scratch::new("serve-banner-verbose");
    let mut server = ServerProcess::start(port, &scratch.db_path(), &["-v"]);

    // Drive a few requests so the access log has rows to print.
    let health = request_http(port, "GET", "/health", None).expect("health request");
    assert_eq!(health.status, 200);
    let created = request_http(port, "POST", "/session", Some("{}")).expect("create session");
    assert_eq!(created.status, 201);

    let (stdout, stderr) = server.stop_and_capture();

    assert!(
        stdout.is_empty(),
        "verbose serve must keep stdout clean, got: {stdout:?}"
    );
    assert!(
        !stderr.contains('\x1b'),
        "piped (non-TTY) stderr must carry no ANSI escape even with -v, got: {stderr:?}"
    );
    let banner = format!("cortex serve listening on http://127.0.0.1:{port}");
    assert!(stderr.contains(&banner), "verbose serve prints the banner:\n{stderr}");
    assert!(
        stderr.contains("routes: /health /event /session /openapi.json"),
        "verbose serve prints the routes hint naming real routes:\n{stderr}"
    );
    assert!(
        stderr.contains("(Ctrl-C to stop)"),
        "routes hint carries the Ctrl-C stop hint:\n{stderr}"
    );

    // Access-log rows for the probed requests, method + path + status columns present.
    let health_row = stderr
        .lines()
        .find(|line| line.trim_start().starts_with("GET") && line.contains("/health"))
        .unwrap_or_else(|| panic!("expected a GET /health access row in:\n{stderr}"));
    assert!(
        health_row.contains("200"),
        "the /health row shows its 200 status: {health_row:?}"
    );
    let session_row = stderr
        .lines()
        .find(|line| line.trim_start().starts_with("POST") && line.contains("/session"))
        .unwrap_or_else(|| panic!("expected a POST /session access row in:\n{stderr}"));
    assert!(
        session_row.contains("201"),
        "the POST /session row shows its 201 status: {session_row:?}"
    );

    // Ordering matches serve_banner.png: the faint routes hint is emitted AFTER the per-request
    // access rows (the access log streams during the run; the hint prints on graceful shutdown).
    let routes_idx = stderr
        .find("routes:")
        .unwrap_or_else(|| panic!("expected the routes hint in:\n{stderr}"));
    let last_access_idx = stderr
        .lines()
        .filter(|line| {
            let t = line.trim_start();
            (t.starts_with("GET") || t.starts_with("POST")) && line.contains('/')
        })
        .map(|line| stderr.find(line).expect("access row is a substring of stderr"))
        .max()
        .unwrap_or_else(|| panic!("expected at least one access row in:\n{stderr}"));
    assert!(
        routes_idx > last_access_idx,
        "the routes hint must follow the access rows (serve_banner.png ordering):\n{stderr}"
    );
}

#[test]
fn http_behavior_is_identical_with_and_without_the_access_log() {
    let _guard = SERVE_BANNER_LOCK.lock().expect("serve banner lock");

    let plain = probe_core_routes(&["-v"], "serve-http-verbose");
    let verbose = probe_core_routes(&[], "serve-http-plain");

    assert_eq!(
        plain.health_status, verbose.health_status,
        "GET /health status must be unaffected by the access log"
    );
    assert_eq!(plain.health_status, 200);
    assert_eq!(
        plain.health_body, verbose.health_body,
        "GET /health body must be unaffected by the access log"
    );
    assert_eq!(
        plain.openapi_status, verbose.openapi_status,
        "GET /openapi.json status must be unaffected by the access log"
    );
    assert_eq!(plain.openapi_status, 200);
    assert_eq!(
        plain.session_status, verbose.session_status,
        "POST /session status must be unaffected by the access log"
    );
    assert_eq!(plain.session_status, 201);
    assert!(
        plain.openapi_body.contains("openapi") && verbose.openapi_body.contains("openapi"),
        "the OpenAPI document is served in both modes"
    );
}

struct CoreRouteProbe {
    health_status: u16,
    health_body: String,
    openapi_status: u16,
    openapi_body: String,
    session_status: u16,
}

fn probe_core_routes(extra_args: &[&str], label: &str) -> CoreRouteProbe {
    let port = free_port();
    let scratch = Scratch::new(label);
    let mut server = ServerProcess::start(port, &scratch.db_path(), extra_args);

    let health = request_http(port, "GET", "/health", None).expect("health request");
    let openapi = request_http(port, "GET", "/openapi.json", None).expect("openapi request");
    let session = request_http(port, "POST", "/session", Some("{}")).expect("create session");

    let _ = server.stop_and_capture();
    CoreRouteProbe {
        health_status: health.status,
        health_body: health.body,
        openapi_status: openapi.status,
        openapi_body: openapi.body,
        session_status: session.status,
    }
}

struct ServerProcess {
    child: Child,
    port: u16,
    stopped: bool,
}

impl ServerProcess {
    fn start(port: u16, db_path: &Path, extra_args: &[&str]) -> Self {
        let port_arg = port.to_string();
        let mut args = vec!["serve", "--hostname", "127.0.0.1", "--port", port_arg.as_str()];
        args.extend_from_slice(extra_args);
        let mut command = Command::new(env!("CARGO_BIN_EXE_cortex"));
        command.args(&args);
        command.current_dir(env!("CARGO_MANIFEST_DIR"));
        command.stdin(Stdio::null());
        command.stdout(Stdio::piped());
        command.stderr(Stdio::piped());
        command
            .env_remove("RUST_LOG")
            .env_remove("CORTEX_VERBOSE")
            .env_remove("OPENROUTER_API_KEY")
            .env_remove("NO_COLOR")
            .env("CORTEX_NO_NET", "1")
            .env("CORTEX_DB_PATH", db_path);
        let mut child = command.spawn().expect("spawn cortex serve");
        wait_for_health(port, &mut child);
        Self {
            child,
            port,
            stopped: false,
        }
    }

    /// Stop the server (SIGTERM the listener) and return its captured (stdout, stderr).
    fn stop_and_capture(&mut self) -> (String, String) {
        if !self.stopped {
            if let Some(pid) = single_listener_pid(self.port) {
                kill_pid(pid);
            }
            wait_for_child_exit(&mut self.child, WAIT_TIMEOUT);
            self.stopped = true;
        }
        let mut stdout = String::new();
        if let Some(mut handle) = self.child.stdout.take() {
            let _ = handle.read_to_string(&mut stdout);
        }
        let mut stderr = String::new();
        if let Some(mut handle) = self.child.stderr.take() {
            let _ = handle.read_to_string(&mut stderr);
        }
        (stdout, stderr)
    }
}

impl Drop for ServerProcess {
    fn drop(&mut self) {
        if !self.stopped {
            if let Some(pid) = single_listener_pid(self.port) {
                kill_pid(pid);
            }
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

struct HttpResponse {
    status: u16,
    body: String,
}

fn wait_for_health(port: u16, child: &mut Child) {
    let started = Instant::now();
    while started.elapsed() < WAIT_TIMEOUT {
        if let Some(status) = child.try_wait().expect("try wait") {
            panic!("server exited before health on port {port}: {status}");
        }
        if let Ok(response) = request_http(port, "GET", "/health", None) {
            if response.status == 200 {
                return;
            }
        }
        thread::sleep(Duration::from_millis(50));
    }
    panic!("server on port {port} did not become healthy");
}

fn request_http(
    port: u16,
    method: &str,
    path: &str,
    body: Option<&str>,
) -> std::io::Result<HttpResponse> {
    let addr: SocketAddr = format!("127.0.0.1:{port}").parse().expect("addr");
    let mut stream = TcpStream::connect_timeout(&addr, Duration::from_millis(250))?;
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    stream.set_write_timeout(Some(Duration::from_secs(2)))?;
    let body = body.unwrap_or("");
    let mut request =
        format!("{method} {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n");
    if !body.is_empty() {
        request.push_str("Content-Type: application/json\r\n");
        request.push_str(&format!("Content-Length: {}\r\n", body.len()));
    }
    request.push_str("\r\n");
    request.push_str(body);
    stream.write_all(request.as_bytes())?;

    let mut raw = String::new();
    stream.read_to_string(&mut raw)?;
    let (head, body) = raw.split_once("\r\n\r\n").unwrap_or((raw.as_str(), ""));
    let status = head
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|code| code.parse().ok())
        .unwrap_or(0);
    Ok(HttpResponse {
        status,
        body: body.to_owned(),
    })
}

fn free_port() -> u16 {
    for port in (server::MIN_PORT..=server::MAX_PORT).rev() {
        if listener_pids(port).is_empty() && TcpListener::bind(("127.0.0.1", port)).is_ok() {
            return port;
        }
    }
    panic!("no free port in 3200-3299");
}

fn listener_pids(port: u16) -> Vec<u32> {
    let output = Command::new("lsof")
        .arg(format!("-tiTCP:{port}"))
        .arg("-sTCP:LISTEN")
        .output()
        .expect("run lsof");
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| line.trim().parse().ok())
        .collect()
}

fn single_listener_pid(port: u16) -> Option<u32> {
    listener_pids(port).into_iter().next()
}

fn kill_pid(pid: u32) {
    let _ = Command::new("kill").arg(pid.to_string()).status();
}

fn wait_for_child_exit(child: &mut Child, timeout: Duration) {
    let started = Instant::now();
    loop {
        if child.try_wait().expect("try wait").is_some() {
            return;
        }
        if started.elapsed() > timeout {
            let _ = child.kill();
            let _ = child.wait();
            return;
        }
        thread::sleep(Duration::from_millis(50));
    }
}

struct Scratch {
    dir: PathBuf,
}

impl Scratch {
    fn new(label: &str) -> Self {
        let id = SCRATCH_COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("cortex-{label}-{}-{id}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("create scratch dir");
        Self { dir }
    }

    fn db_path(&self) -> PathBuf {
        self.dir.join("cortex.db")
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}
