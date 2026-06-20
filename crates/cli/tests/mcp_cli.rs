//! Black-box checks for the `cortex mcp` command surface that exercise the remote MCP transport
//! and the MCP-remote OAuth credential lifecycle through the real CLI binary.

use std::fs;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

use serde_json::Value;

const RUN_TIMEOUT: Duration = Duration::from_secs(20);

fn cortex_bin() -> String {
    env!("CARGO_BIN_EXE_cortex").to_owned()
}

fn remote_server_bin() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let bin = exe.parent()?.parent()?.join("cortex-mcp-remote-server");
    bin.exists().then_some(bin)
}

fn echo_server_bin() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let bin = exe.parent()?.parent()?.join("cortex-mcp-echo-server");
    bin.exists().then_some(bin)
}

fn toml_string(value: &Path) -> String {
    value
        .to_string_lossy()
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
}

fn temp_path(tag: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "cortex-mcp-cli-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ))
}

struct Scratch {
    root: PathBuf,
}

impl Scratch {
    fn new(tag: &str) -> Self {
        let root = temp_path(tag);
        fs::create_dir_all(root.join("home/proj")).expect("scratch project");
        fs::create_dir_all(root.join("data")).expect("scratch data");
        Self { root }
    }

    fn home(&self) -> PathBuf {
        self.root.join("home")
    }

    fn project(&self) -> PathBuf {
        self.root.join("home/proj")
    }

    fn data(&self) -> PathBuf {
        self.root.join("data")
    }

    fn write_project_config(&self, body: &str) {
        fs::write(self.project().join("cortex.toml"), body).expect("write cortex.toml");
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

struct Fixture {
    child: Child,
}

impl Fixture {
    fn spawn(mode: &str, port: u16, log: &Path) -> Option<Self> {
        let bin = remote_server_bin()?;
        let mut command = Command::new(bin);
        command
            .env("CORTEX_MCP_MODE", mode)
            .env("CORTEX_MCP_PORT", port.to_string())
            .env("CORTEX_MCP_LOG", log)
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let child = command.spawn().expect("spawn remote MCP fixture");
        wait_for_port(port);
        Some(Self { child })
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn wait_for_port(port: u16) {
    let start = Instant::now();
    while start.elapsed() < RUN_TIMEOUT {
        if TcpStream::connect(("127.0.0.1", port)).is_ok() {
            return;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    panic!("fixture did not listen on 127.0.0.1:{port}");
}

fn run_command(mut command: Command) -> Output {
    command.stdin(Stdio::null());
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());

    let mut child = command.spawn().expect("spawn command");
    let mut out = child.stdout.take().expect("stdout handle");
    let mut err = child.stderr.take().expect("stderr handle");
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
        if start.elapsed() > RUN_TIMEOUT {
            let _ = child.kill();
            let _ = child.wait();
            panic!("command did not exit within {RUN_TIMEOUT:?}");
        }
        std::thread::sleep(Duration::from_millis(20));
    };

    Output {
        status,
        stdout: out_thread.join().expect("join stdout reader"),
        stderr: err_thread.join().expect("join stderr reader"),
    }
}

fn cortex_command(scratch: &Scratch, args: &[&str]) -> Command {
    let mut command = Command::new(cortex_bin());
    command
        .args(args)
        .env_clear()
        .env("HOME", scratch.home())
        .env("CORTEX_DATA_DIR", scratch.data())
        .current_dir(scratch.project());
    command
}

fn mcp_list_json(scratch: &Scratch) -> Value {
    let output = run_command(cortex_command(
        scratch,
        &["mcp", "list", "--format", "json"],
    ));
    assert_eq!(
        output.status.code(),
        Some(0),
        "mcp list failed; stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("mcp list JSON")
}

fn read_first_json_line(reader: &mut impl Read) -> Value {
    let mut line = Vec::new();
    let start = Instant::now();
    while start.elapsed() < RUN_TIMEOUT {
        let mut byte = [0u8; 1];
        match reader.read(&mut byte) {
            Ok(0) => break,
            Ok(_) if byte[0] == b'\n' => break,
            Ok(_) => line.push(byte[0]),
            Err(err) => panic!("read auth stdout: {err}"),
        }
    }
    serde_json::from_slice(&line).unwrap_or_else(|err| {
        panic!(
            "auth command should emit a JSON document first: {err}; line: {}",
            String::from_utf8_lossy(&line)
        )
    })
}

fn http_get_follow_once(url: &str) -> String {
    let first = http_get(url);
    if first.status == 302 {
        let location = first
            .headers
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case("location"))
            .map(|(_, value)| value.clone())
            .expect("redirect location");
        return http_get(&location).body;
    }
    first.body
}

struct HttpResponse {
    status: u16,
    headers: Vec<(String, String)>,
    body: String,
}

fn http_get(url: &str) -> HttpResponse {
    let rest = url
        .strip_prefix("http://")
        .expect("only loopback http URLs");
    let (authority, target) = match rest.split_once('/') {
        Some((authority, path)) => (authority, format!("/{path}")),
        None => (rest, "/".to_owned()),
    };
    let (host, port) = authority
        .rsplit_once(':')
        .map(|(host, port)| (host, port.parse::<u16>().expect("port")))
        .unwrap_or((authority, 80));
    let mut stream = TcpStream::connect((host, port)).expect("connect http server");
    stream
        .write_all(
            format!("GET {target} HTTP/1.1\r\nHost: {authority}\r\nConnection: close\r\n\r\n")
                .as_bytes(),
        )
        .expect("write http request");
    let mut raw_bytes = Vec::new();
    loop {
        let mut buffer = [0u8; 1024];
        match stream.read(&mut buffer) {
            Ok(0) => break,
            Ok(n) => raw_bytes.extend_from_slice(&buffer[..n]),
            Err(err) if err.kind() == std::io::ErrorKind::ConnectionReset => break,
            Err(err) => panic!("read http response: {err}"),
        }
    }
    let raw = String::from_utf8_lossy(&raw_bytes).into_owned();
    let (head, body) = raw.split_once("\r\n\r\n").unwrap_or((raw.as_str(), ""));
    let mut lines = head.split("\r\n");
    let status = lines
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|code| code.parse::<u16>().ok())
        .expect("status code");
    let headers = lines
        .filter_map(|line| {
            let (name, value) = line.split_once(':')?;
            Some((name.trim().to_owned(), value.trim().to_owned()))
        })
        .collect();
    HttpResponse {
        status,
        headers,
        body: body.to_owned(),
    }
}

#[test]
fn mcp_list_connects_remote_streamable_and_sse_fallback() {
    let Some(_bin) = remote_server_bin() else {
        eprintln!("skipping: cortex-mcp-remote-server fixture not built");
        return;
    };
    let scratch = Scratch::new("list");
    let http_log = scratch.root.join("http.log");
    let sse_log = scratch.root.join("sse.log");
    let _http = Fixture::spawn("streamable", 3224, &http_log).expect("http fixture");
    let _sse = Fixture::spawn("sse", 3225, &sse_log).expect("sse fixture");
    scratch.write_project_config(
        r#"
[mcp.http]
type = "remote"
url = "http://127.0.0.1:3224/mcp"

[mcp.legacy]
type = "remote"
url = "http://127.0.0.1:3225/sse"
"#,
    );

    let list = mcp_list_json(&scratch);
    let servers = list["servers"].as_array().expect("servers array");
    let by_name = |name: &str| {
        servers
            .iter()
            .find(|server| server["name"] == name)
            .unwrap_or_else(|| panic!("missing server {name}: {list}"))
    };

    let http = by_name("http");
    assert_eq!(http["status"], "connected");
    assert!(
        http["tools"]
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t["id"] == "http_add")
    );
    let legacy = by_name("legacy");
    assert_eq!(legacy["status"], "connected");
    assert!(
        legacy["tools"]
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t["id"] == "legacy_echo")
    );

    assert!(
        fs::read_to_string(&http_log)
            .unwrap_or_default()
            .contains("HTTP MCP session established (streamable)")
    );
    assert!(
        fs::read_to_string(&sse_log)
            .unwrap_or_default()
            .contains("SSE endpoint opened")
    );
}

#[test]
fn mcp_auth_authorizes_persists_reuses_and_does_not_leak_token() {
    let Some(_bin) = remote_server_bin() else {
        eprintln!("skipping: cortex-mcp-remote-server fixture not built");
        return;
    };
    let scratch = Scratch::new("oauth");
    let log = scratch.root.join("oauth.log");
    let _fixture = Fixture::spawn("oauth", 3226, &log).expect("oauth fixture");
    scratch.write_project_config(
        r#"
[mcp.api]
type = "remote"
url = "http://127.0.0.1:3226/mcp"
"#,
    );

    let before = mcp_list_json(&scratch);
    assert_eq!(before["servers"][0]["status"], "needs_auth");

    let mut auth = cortex_command(
        &scratch,
        &[
            "mcp",
            "auth",
            "api",
            "--callback-port",
            "3227",
            "--timeout",
            "10",
            "--format",
            "json",
        ],
    );
    let mut child = auth
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn mcp auth");
    let mut stdout = child.stdout.take().expect("auth stdout");
    let mut stderr = child.stderr.take().expect("auth stderr");
    let start = read_first_json_line(&mut stdout);
    let authorize_url = start["authorize_url"]
        .as_str()
        .expect("authorize_url")
        .to_owned();

    let callback_body = http_get_follow_once(&authorize_url);
    assert!(
        callback_body.contains("Authorization complete"),
        "callback body: {callback_body}"
    );

    let status = child.wait().expect("wait mcp auth");
    assert_eq!(status.code(), Some(0), "mcp auth must exit successfully");
    let mut auth_stdout_rest = Vec::new();
    let mut auth_stderr = Vec::new();
    let _ = stdout.read_to_end(&mut auth_stdout_rest);
    let _ = stderr.read_to_end(&mut auth_stderr);

    let store = scratch.data().join("mcp-oauth.json");
    let mode = fs::metadata(&store)
        .expect("credential store exists")
        .permissions();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(mode.mode() & 0o777, 0o600, "store must be 0600");
    }

    let after = mcp_list_json(&scratch);
    assert_eq!(after["servers"][0]["status"], "connected");
    assert!(
        after["servers"][0]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t["id"] == "api_add")
    );

    let store_body = fs::read_to_string(&store).expect("read credential store");
    let token =
        serde_json::from_str::<Value>(&store_body).expect("credential JSON")["api"]["access_token"]
            .as_str()
            .expect("access token")
            .to_owned();
    let mut observed = Vec::new();
    observed.extend_from_slice(start.to_string().as_bytes());
    observed.extend_from_slice(&auth_stdout_rest);
    observed.extend_from_slice(&auth_stderr);
    observed.extend_from_slice(before.to_string().as_bytes());
    observed.extend_from_slice(after.to_string().as_bytes());
    observed.extend_from_slice(fs::read_to_string(&log).unwrap_or_default().as_bytes());
    assert!(
        !String::from_utf8_lossy(&observed).contains(&token),
        "OAuth access token must never appear in CLI output or logs"
    );
}

#[test]
fn mcp_list_reports_resources_and_prompts() {
    let Some(bin) = echo_server_bin() else {
        eprintln!("skipping: cortex-mcp-echo-server fixture not built");
        return;
    };
    let scratch = Scratch::new("list-resources-prompts");
    scratch.write_project_config(&format!(
        r#"
[mcp.demo]
type = "local"
command = ["{}"]
[mcp.demo.environment]
CORTEX_MCP_ECHO_ENABLE_RESOURCES_PROMPTS = "1"
CORTEX_MCP_ECHO_RESOURCE_CONTENT = "RESOURCE-CONTENT-CLI"
"#,
        toml_string(&bin)
    ));

    let list = mcp_list_json(&scratch);
    let server = &list["servers"][0];
    assert_eq!(server["status"], "connected");
    assert!(
        server["tools"]
            .as_array()
            .unwrap()
            .iter()
            .any(|tool| tool["id"] == "demo_add"),
        "tools are still listed: {list}"
    );
    assert!(
        server["resources"]
            .as_array()
            .unwrap()
            .iter()
            .any(|resource| resource["uri"] == "notes://demo"),
        "resources must be enumerated in mcp list JSON: {list}"
    );
    assert!(
        server["prompts"]
            .as_array()
            .unwrap()
            .iter()
            .any(|prompt| prompt["name"] == "greet"),
        "prompts must be enumerated in mcp list JSON: {list}"
    );
}
