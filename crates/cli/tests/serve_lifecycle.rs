use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant};

static SERVE_TEST_LOCK: Mutex<()> = Mutex::new(());
static SCRATCH_COUNTER: AtomicU64 = AtomicU64::new(0);
const WAIT_TIMEOUT: Duration = Duration::from_secs(8);

#[test]
fn serve_health_loopback_bind_and_shutdown_release_are_black_box_visible() {
    let _guard = SERVE_TEST_LOCK.lock().expect("serve test lock");
    let port = free_port();
    let mut server = ServerProcess::start(port);

    let response = wait_for_health(port, &mut server.child);
    assert_eq!(response.status, 200);
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&response.body).expect("health JSON"),
        serde_json::json!({"status": "ok"})
    );

    let pids = listener_pids(port);
    assert_eq!(pids, vec![server.child.id()]);
    let details = listener_details(port);
    assert!(
        details.contains(&format!("127.0.0.1:{port}")),
        "listener details should show canonical loopback address, got:\n{details}"
    );
    assert!(
        !details.contains(&format!("0.0.0.0:{port}")) && !details.contains(&format!("*:{port}")),
        "listener must not bind a wildcard address:\n{details}"
    );

    server.stop();
    wait_for_port_released(port);

    let mut relaunched = ServerProcess::start(port);
    let relaunched_response = wait_for_health(port, &mut relaunched.child);
    assert_eq!(relaunched_response.status, 200);
    relaunched.stop();
    wait_for_port_released(port);
}

#[test]
fn serve_session_crud_routes_are_black_box_visible_and_db_backed() {
    let _guard = SERVE_TEST_LOCK.lock().expect("serve test lock");
    let port = free_port();
    let scratch = Scratch::new("serve-session-crud");
    let db_path = scratch.db_path();
    let mut server = ServerProcess::start_with_db_path(port, &db_path);

    let created_response = request_json(port, "POST", "/session", Some("{}"));
    assert_eq!(created_response.status, 201);
    assert_json_content_type(&created_response);
    let created: db::Session =
        serde_json::from_str(&created_response.body).expect("created session JSON");
    assert!(
        !created.id.is_empty(),
        "created session id must be non-empty"
    );

    let list_response = request_json(port, "GET", "/session", None);
    assert_eq!(list_response.status, 200);
    assert_json_content_type(&list_response);
    let listed: Vec<db::Session> =
        serde_json::from_str(&list_response.body).expect("session list JSON");
    assert!(
        listed.iter().any(|session| session.id == created.id),
        "freshly created session should be listed: {listed:?}"
    );

    let get_response = request_json(port, "GET", &format!("/session/{}", created.id), None);
    assert_eq!(get_response.status, 200);
    assert_json_content_type(&get_response);
    let fetched: db::Session =
        serde_json::from_str(&get_response.body).expect("fetched session JSON");
    assert_eq!(fetched.id, created.id);

    let delete_response = request_json(port, "DELETE", &format!("/session/{}", created.id), None);
    assert_eq!(delete_response.status, 204);

    let deleted_get = request_json(port, "GET", &format!("/session/{}", created.id), None);
    assert_eq!(deleted_get.status, 404);
    assert_json_content_type(&deleted_get);

    let list_after_delete = request_json(port, "GET", "/session", None);
    assert_eq!(list_after_delete.status, 200);
    let listed_after_delete: Vec<db::Session> =
        serde_json::from_str(&list_after_delete.body).expect("post-delete session list JSON");
    assert!(
        listed_after_delete
            .iter()
            .all(|session| session.id != created.id),
        "deleted session should be absent from list: {listed_after_delete:?}"
    );

    let persisted_response = request_json(port, "POST", "/session", Some("{}"));
    assert_eq!(persisted_response.status, 201);
    let persisted: db::Session =
        serde_json::from_str(&persisted_response.body).expect("persisted session JSON");
    assert_ne!(persisted.id, created.id);

    server.stop();
    wait_for_port_released(port);

    let mut restarted = ServerProcess::start_with_db_path(port, &db_path);
    let restarted_list_response = request_json(port, "GET", "/session", None);
    assert_eq!(restarted_list_response.status, 200);
    let restarted_list: Vec<db::Session> =
        serde_json::from_str(&restarted_list_response.body).expect("restarted list JSON");
    assert!(
        restarted_list
            .iter()
            .any(|session| session.id == persisted.id),
        "session created before restart should be persisted and listed after restart: {restarted_list:?}"
    );
    let cleanup_delete = request_json(port, "DELETE", &format!("/session/{}", persisted.id), None);
    assert_eq!(cleanup_delete.status, 204);
    restarted.stop();
    wait_for_port_released(port);

    let db = db::Database::open(&db_path).expect("open server db after shutdown");
    assert!(
        db.sessions()
            .get(&created.id)
            .expect("query deleted session")
            .is_none(),
        "deleted session should be absent from persisted db"
    );
    assert!(
        db.sessions()
            .get(&persisted.id)
            .expect("query cleanup session")
            .is_none(),
        "cleanup-deleted restarted session should be absent from persisted db"
    );
}

#[test]
fn serve_busy_port_exits_nonzero_with_clear_diagnostic() {
    let _guard = SERVE_TEST_LOCK.lock().expect("serve test lock");
    let port = free_port();
    let mut server = ServerProcess::start(port);
    assert_eq!(wait_for_health(port, &mut server.child).status, 200);
    let scratch = Scratch::new("serve-busy-command");

    let mut command = cortex_command(&[
        "serve",
        "--hostname",
        "127.0.0.1",
        "--port",
        &port.to_string(),
    ]);
    command.env("CORTEX_DB_PATH", scratch.db_path());
    let output = run_to_output(command, WAIT_TIMEOUT);

    assert!(
        !output.status.success(),
        "busy serve unexpectedly succeeded"
    );
    assert!(
        output.stdout.is_empty(),
        "busy-port failure should not write stdout, got: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("already in use"), "{stderr}");
    assert!(stderr.contains(&port.to_string()), "{stderr}");
    assert!(!stderr.contains("panicked at"), "{stderr}");
    assert!(!stderr.contains("RUST_BACKTRACE"), "{stderr}");
    assert_eq!(wait_for_health(port, &mut server.child).status, 200);

    server.stop();
    wait_for_port_released(port);
}

struct ServerProcess {
    child: Child,
    port: u16,
    stopped: bool,
    _scratch: Option<Scratch>,
}

impl ServerProcess {
    fn start(port: u16) -> Self {
        let scratch = Scratch::new("serve-process");
        let db_path = scratch.db_path();
        Self::start_with_db_path_and_scratch(port, &db_path, Some(scratch))
    }

    fn start_with_db_path(port: u16, db_path: &Path) -> Self {
        Self::start_with_db_path_and_scratch(port, db_path, None)
    }

    fn start_with_db_path_and_scratch(port: u16, db_path: &Path, scratch: Option<Scratch>) -> Self {
        let mut command = cortex_command(&[
            "serve",
            "--hostname",
            "127.0.0.1",
            "--port",
            &port.to_string(),
        ]);
        command.env("CORTEX_DB_PATH", db_path);
        let mut child = command.spawn().expect("spawn cortex serve");
        let _ = wait_for_health(port, &mut child);
        Self {
            child,
            port,
            stopped: false,
            _scratch: scratch,
        }
    }

    fn stop(&mut self) {
        if self.stopped {
            return;
        }
        if let Some(pid) = single_listener_pid(self.port) {
            kill_pid(pid, None);
        }
        wait_for_child_exit(&mut self.child, WAIT_TIMEOUT);
        self.stopped = true;
    }
}

impl Drop for ServerProcess {
    fn drop(&mut self) {
        self.stop();
    }
}

struct HttpResponse {
    status: u16,
    headers: String,
    body: String,
}

fn cortex_command(args: &[&str]) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_cortex"));
    command.args(args);
    command.current_dir(env!("CARGO_MANIFEST_DIR"));
    command.stdin(Stdio::null());
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());
    command
        .env_remove("RUST_LOG")
        .env_remove("CORTEX_VERBOSE")
        .env_remove("CORTEX_NO_NET")
        .env_remove("CORTEX_DB_PATH")
        .env_remove("CORTEX_DATA_DIR");
    command
}

fn wait_for_health(port: u16, child: &mut Child) -> HttpResponse {
    let started = Instant::now();
    let mut last_error = String::new();
    while started.elapsed() < WAIT_TIMEOUT {
        if let Some(status) = child.try_wait().expect("try wait") {
            panic!("server exited before health on port {port}: {status}");
        }
        match request_health(port) {
            Ok(response) if response.status == 200 => return response,
            Ok(response) => {
                last_error = format!("HTTP {}", response.status);
            }
            Err(err) => {
                last_error = err.to_string();
            }
        }
        thread::sleep(Duration::from_millis(50));
    }
    panic!("server on port {port} did not become healthy: {last_error}");
}

fn request_health(port: u16) -> std::io::Result<HttpResponse> {
    request_http(port, "GET", "/health", None)
}

fn request_json(port: u16, method: &str, path: &str, body: Option<&str>) -> HttpResponse {
    request_http(port, method, path, body).unwrap_or_else(|err| {
        panic!("HTTP request {method} {path} to port {port} failed: {err}");
    })
}

fn request_http(
    port: u16,
    method: &str,
    path: &str,
    body: Option<&str>,
) -> std::io::Result<HttpResponse> {
    let addr: SocketAddr = format!("127.0.0.1:{port}").parse().expect("addr");
    let mut stream = TcpStream::connect_timeout(&addr, Duration::from_millis(250))?;
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    stream.set_write_timeout(Some(Duration::from_secs(2)))?;
    let body = body.unwrap_or("");
    let request = if body.is_empty() {
        format!("{method} {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n")
    } else {
        format!(
            "{method} {path} HTTP/1.1\r\n\
             Host: 127.0.0.1\r\n\
             Connection: close\r\n\
             Content-Type: application/json\r\n\
             Content-Length: {}\r\n\r\n\
             {body}",
            body.len()
        )
    };
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
        headers: head.to_owned(),
        body: body.to_owned(),
    })
}

fn assert_json_content_type(response: &HttpResponse) {
    let headers = response.headers.to_ascii_lowercase();
    assert!(
        headers.contains("content-type: application/json"),
        "expected application/json response, got headers:\n{}",
        response.headers
    );
}

fn run_to_output(mut command: Command, timeout: Duration) -> Output {
    let mut child = command.spawn().expect("spawn command");
    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().expect("try wait") {
            break status;
        }
        if started.elapsed() > timeout {
            let _ = child.kill();
            let _ = child.wait();
            panic!("command did not exit within {timeout:?}");
        }
        thread::sleep(Duration::from_millis(25));
    };

    let mut stdout = Vec::new();
    if let Some(mut handle) = child.stdout.take() {
        handle.read_to_end(&mut stdout).expect("read stdout");
    }
    let mut stderr = Vec::new();
    if let Some(mut handle) = child.stderr.take() {
        handle.read_to_end(&mut stderr).expect("read stderr");
    }
    Output {
        status,
        stdout,
        stderr,
    }
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
    let pids = listener_pids(port);
    match pids.as_slice() {
        [pid] => Some(*pid),
        [] => None,
        many => panic!("expected a single listener pid for {port}, got {many:?}"),
    }
}

fn listener_details(port: u16) -> String {
    let output = Command::new("lsof")
        .arg("-nP")
        .arg(format!("-iTCP:{port}"))
        .arg("-sTCP:LISTEN")
        .output()
        .expect("run lsof details");
    assert!(
        output.status.success(),
        "lsof details failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn kill_pid(pid: u32, signal: Option<&str>) {
    let mut command = Command::new("kill");
    if let Some(signal) = signal {
        command.arg(signal);
    }
    let status = command.arg(pid.to_string()).status().expect("run kill");
    assert!(status.success(), "kill {pid} failed with {status}");
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
            panic!("server child did not exit within {timeout:?}");
        }
        thread::sleep(Duration::from_millis(50));
    }
}

fn wait_for_port_released(port: u16) {
    let started = Instant::now();
    while started.elapsed() < WAIT_TIMEOUT {
        if listener_pids(port).is_empty() {
            assert!(
                TcpListener::bind(("127.0.0.1", port)).is_ok(),
                "port {port} has no lsof listener but cannot be rebound"
            );
            return;
        }
        thread::sleep(Duration::from_millis(50));
    }
    panic!("port {port} still has a listener after shutdown");
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
