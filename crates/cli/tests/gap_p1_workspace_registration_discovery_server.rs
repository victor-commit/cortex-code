use std::fs;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::Value;

const WAIT_TIMEOUT: Duration = Duration::from_secs(10);

#[test]
fn serve_discovery_routes_include_plugin_workspace_registrations() {
    let scratch = Scratch::new("gap-p1-workspace-discovery");
    let workspace = scratch.root.join("workspace");
    fs::create_dir_all(&workspace).expect("create workspace");
    write_plugin(&workspace);

    let port = free_port();
    let mut child = spawn_serve(&scratch, &workspace, port);
    wait_for_health(port, &mut child);
    let listener = listener_pid(port).expect("serve listener pid");
    assert_eq!(
        listener,
        child.id(),
        "serve listener should be owned by the spawned process"
    );
    let _guard = ServeGuard::new(child, port, listener);

    let project_discovery =
        request_http(port, "GET", "/project/discovery", None).expect("project route");
    assert_eq!(
        project_discovery.status, 200,
        "project discovery status/body: {}\n{}",
        project_discovery.status, project_discovery.body
    );
    let project_json: Value = serde_json::from_str(&project_discovery.body).expect("project JSON");
    let project_adapters = project_json["workspace_adapters"]
        .as_array()
        .expect("project workspace adapters array");
    let project_match = project_adapters
        .iter()
        .find(|entry| entry["type"] == "folder")
        .expect("project discovery adapter registration");
    assert_eq!(project_match["plugin"], "./plugin.mjs");
    assert_eq!(project_match["name"], "Folder adapter");
    assert_eq!(
        project_match["description"],
        "Workspace adapter from discovery route test"
    );

    let workspace_discovery = request_http(port, "GET", "/experimental/workspace/adapter", None)
        .expect("workspace route");
    assert_eq!(
        workspace_discovery.status, 200,
        "workspace discovery status/body: {}\n{}",
        workspace_discovery.status, workspace_discovery.body
    );
    let workspace_json: Value =
        serde_json::from_str(&workspace_discovery.body).expect("workspace JSON");
    let workspace_adapters = workspace_json.as_array().expect("workspace adapters array");
    assert!(
        workspace_adapters.iter().any(|entry| {
            entry["plugin"] == "./plugin.mjs"
                && entry["type"] == "folder"
                && entry["name"] == "Folder adapter"
                && entry["description"] == "Workspace adapter from discovery route test"
        }),
        "workspace discovery should include plugin registration: {workspace_json:#}"
    );

    let openapi = request_http(port, "GET", "/openapi.json", None).expect("openapi route");
    assert_eq!(openapi.status, 200);
    let openapi_json: Value = serde_json::from_str(&openapi.body).expect("openapi JSON");
    assert!(
        openapi_json["paths"]["/project/discovery"]["get"].is_object(),
        "openapi should document GET /project/discovery"
    );
    assert!(
        openapi_json["paths"]["/experimental/workspace/adapter"]["get"].is_object(),
        "openapi should document GET /experimental/workspace/adapter"
    );
    assert_eq!(
        openapi_json["components"]["schemas"]["WorkspaceAdapterDiscovery"]["properties"]["type"]["type"],
        "string",
        "workspace adapter schema should expose `type`"
    );
}

fn write_plugin(workspace: &Path) {
    fs::write(
        workspace.join("plugin.mjs"),
        r#"
export default async function plugin(input) {
  input.experimental_workspace.register("folder", {
    name: "Folder adapter",
    description: "Workspace adapter from discovery route test"
  });
  return {};
}
"#,
    )
    .expect("write plugin");
    fs::write(
        workspace.join("cortex.toml"),
        "plugin = [\"./plugin.mjs\"]\n",
    )
    .expect("write plugin config");
}

fn spawn_serve(scratch: &Scratch, cwd: &Path, port: u16) -> Child {
    let mut command = Command::new(env!("CARGO_BIN_EXE_cortex"));
    command.current_dir(cwd);
    command.args([
        "serve",
        "--hostname",
        "127.0.0.1",
        "--port",
        &port.to_string(),
    ]);
    command.env("HOME", &scratch.home);
    command.env("CORTEX_DB_PATH", &scratch.db);
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("XDG_DATA_HOME")
        .env_remove("CORTEX_DATA_DIR")
        .env_remove("CORTEX_NO_NET");
    command.spawn().expect("spawn serve")
}

fn wait_for_health(port: u16, child: &mut Child) {
    let started = Instant::now();
    let mut last_error = String::new();
    while started.elapsed() < WAIT_TIMEOUT {
        if let Some(status) = child.try_wait().expect("serve try_wait") {
            panic!("serve exited before health check: {status}");
        }
        match request_http(port, "GET", "/health", None) {
            Ok(response) if response.status == 200 => return,
            Ok(response) => last_error = format!("status {}", response.status),
            Err(err) => last_error = err.to_string(),
        }
        thread::sleep(Duration::from_millis(50));
    }
    panic!("serve never became healthy: {last_error}");
}

struct HttpResponse {
    status: u16,
    body: String,
}

fn request_http(
    port: u16,
    method: &str,
    path: &str,
    body: Option<&str>,
) -> std::io::Result<HttpResponse> {
    let addr: SocketAddr = format!("127.0.0.1:{port}").parse().expect("valid addr");
    let mut stream = TcpStream::connect_timeout(&addr, Duration::from_millis(250))?;
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
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
    let (head, body) = raw
        .split_once("\r\n\r\n")
        .map_or((raw.as_str(), ""), |(head, body)| (head, body));
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
    for port in server::MIN_PORT..=server::MAX_PORT {
        if listener_pid(port).is_none() && TcpListener::bind(("127.0.0.1", port)).is_ok() {
            return port;
        }
    }
    panic!("no free port in 3200-3299");
}

fn listener_pid(port: u16) -> Option<u32> {
    let output = Command::new("lsof")
        .arg(format!("-tiTCP:{port}"))
        .arg("-sTCP:LISTEN")
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .find_map(|line| line.trim().parse().ok())
}

fn wait_for_port_released(port: u16) {
    let started = Instant::now();
    while started.elapsed() < WAIT_TIMEOUT {
        if listener_pid(port).is_none() {
            return;
        }
        thread::sleep(Duration::from_millis(50));
    }
}

struct ServeGuard {
    child: Child,
    port: u16,
    listener_pid: u32,
}

impl ServeGuard {
    fn new(child: Child, port: u16, listener_pid: u32) -> Self {
        Self {
            child,
            port,
            listener_pid,
        }
    }
}

impl Drop for ServeGuard {
    fn drop(&mut self) {
        let _ = Command::new("kill")
            .arg(self.listener_pid.to_string())
            .status();

        let started = Instant::now();
        while started.elapsed() < WAIT_TIMEOUT {
            if self.child.try_wait().ok().flatten().is_some() {
                break;
            }
            thread::sleep(Duration::from_millis(50));
        }
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
        wait_for_port_released(self.port);
    }
}

struct Scratch {
    root: PathBuf,
    home: PathBuf,
    db: PathBuf,
}

impl Scratch {
    fn new(tag: &str) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let root =
            std::env::temp_dir().join(format!("cortex-{tag}-{}-{nanos}", std::process::id()));
        let home = root.join("home");
        fs::create_dir_all(&home).expect("create home");
        let db = root.join("cortex.db");
        Self { root, home, db }
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
