//! Black-box integration coverage for the `/mcp` catalog route (VAL-GAP-CROSS-010).
//!
//! `GET /mcp` connects the configured MCP servers (here a real stdio echo fixture), reports each
//! server's status, and surfaces the flattened resources and prompts with the same server-qualified
//! ids the CLI `cortex mcp list` and the TUI `/mcp` view expose. Connecting also announces the
//! discovered catalogues on the event bus, so an `/event` SSE subscriber observes the same resource
//! and prompt entries. Every spawned stdio child is shut down by exact PID before the route returns.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode, header};
use events::{Event, EventBus};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tower::ServiceExt;

static SCRATCH_COUNTER: AtomicU64 = AtomicU64::new(0);

#[tokio::test]
async fn mcp_route_lists_resources_and_prompts_and_announces_them_on_the_event_bus() {
    let Some(echo) = echo_server_bin() else {
        eprintln!("skipping /mcp route test: cortex-mcp-echo-server fixture is unavailable");
        return;
    };

    let scratch = Scratch::new("server-mcp-catalog");
    let workspace = scratch.workspace();
    let log = scratch.dir.join("mcp.log");
    std::fs::write(
        workspace.join("cortex.toml"),
        format!(
            r#"
[mcp.demo]
type = "local"
command = ["{}", "{}"]
[mcp.demo.environment]
CORTEX_MCP_ECHO_LOG = "{}"
CORTEX_MCP_ECHO_ENABLE_RESOURCES_PROMPTS = "1"
CORTEX_MCP_ECHO_ENABLE_NOARGS_PROMPT = "1"
"#,
            toml_escape(&echo),
            scratch.proc_tag,
            toml_escape(&log),
        ),
    )
    .expect("write cortex.toml");

    let bus = Arc::new(EventBus::new());
    let mut receiver = bus.subscribe_stream();
    let app = server::app_with_state(
        server::ServerState::new(scratch.db_path())
            .with_cwd(workspace.clone())
            .with_config_env(Vec::new())
            .with_event_bus(bus.clone()),
    );

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/mcp")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value = json_body(response).await;

    let servers = body["servers"].as_array().expect("servers array");
    let demo = servers
        .iter()
        .find(|server| server["name"] == "demo")
        .expect("demo server present");
    assert_eq!(
        demo["status"], "connected",
        "stdio fixture should connect: {body:?}"
    );

    let resources = body["resources"].as_array().expect("resources array");
    let demo_resource = resources
        .iter()
        .find(|resource| resource["uri"] == "notes://demo")
        .expect("notes://demo resource present");
    assert_eq!(demo_resource["server"], "demo");
    assert!(
        demo_resource["id"]
            .as_str()
            .is_some_and(|id| !id.is_empty()),
        "resource carries a server-qualified id: {body:?}"
    );

    let prompts = body["prompts"].as_array().expect("prompts array");
    let greet = prompts
        .iter()
        .find(|prompt| prompt["name"] == "greet")
        .expect("greet prompt present");
    assert_eq!(greet["server"], "demo");
    assert!(
        greet["arguments"]
            .as_array()
            .is_some_and(|args| args.iter().any(|arg| arg["name"] == "name")),
        "greet prompt advertises its declared arguments: {body:?}"
    );

    // The same entries are announced on the event bus for `/event` SSE subscribers.
    let mut resource_event_uris: HashSet<String> = HashSet::new();
    let mut prompt_event_names: HashSet<String> = HashSet::new();
    while let Ok(event) = receiver.try_recv() {
        match event {
            Event::McpResourcesChanged(payload) if payload.server == "demo" => {
                resource_event_uris.extend(payload.resources);
            }
            Event::McpPromptsChanged(payload) if payload.server == "demo" => {
                prompt_event_names.extend(payload.prompts);
            }
            _ => {}
        }
    }
    assert!(
        resource_event_uris.contains("notes://demo"),
        "mcp.resources.changed should announce the demo resource: {resource_event_uris:?}"
    );
    assert!(
        prompt_event_names.contains("greet"),
        "mcp.prompts.changed should announce the demo prompt: {prompt_event_names:?}"
    );

    // No orphaned stdio child should survive the ephemeral connect. Scope the scan to this
    // sandbox's unique argv token so concurrent sibling tests' identically-named echo-server
    // fixtures are never misattributed, and poll briefly because the OS may take a moment to reap
    // the killed child (ignoring already-defunct zombie entries).
    let proc_tag = scratch.proc_tag.clone();
    let mut leaked = true;
    for _ in 0..30 {
        if !live_echo_server_running(&proc_tag) {
            leaked = false;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    assert!(
        !leaked,
        "the /mcp route must shut down the stdio fixture by PID, leaving no orphan"
    );
}

/// Whether a live (non-zombie) echo-server process tagged with `proc_tag` is currently running.
fn live_echo_server_running(proc_tag: &str) -> bool {
    let Ok(output) = std::process::Command::new("ps")
        .args(["-eo", "stat,args"])
        .output()
    else {
        return false;
    };
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter(|line| line.contains(proc_tag))
        .any(|line| {
            // The first whitespace-separated field is the process state; `Z` marks an
            // already-reaped zombie that no longer holds resources.
            !line
                .split_whitespace()
                .next()
                .is_some_and(|stat| stat.starts_with('Z'))
        })
}

fn echo_server_bin() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let bin = exe.parent()?.parent()?.join("cortex-mcp-echo-server");
    bin.exists().then_some(bin)
}

fn toml_escape(path: &std::path::Path) -> String {
    path.to_string_lossy()
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
}

async fn json_body<T>(response: axum::response::Response) -> T
where
    T: serde::de::DeserializeOwned,
{
    let bytes = to_bytes(response.into_body(), 262_144).await.expect("body");
    serde_json::from_slice(&bytes).expect("JSON body")
}

#[tokio::test]
async fn mcp_route_surfaces_needs_auth_without_an_oauth_challenge() {
    let scratch = Scratch::new("server-mcp-needs-auth");
    let workspace = scratch.workspace();
    let fixture = NeedsAuthFixture::start().await;
    std::fs::write(
        workspace.join("cortex.toml"),
        format!(
            r#"
[mcp.api]
type = "remote"
url = "{}"
"#,
            fixture.url
        ),
    )
    .expect("write cortex.toml");

    let app = server::app_with_state(
        server::ServerState::new(scratch.db_path())
            .with_cwd(workspace)
            .with_config_env(Vec::new()),
    );
    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/mcp")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");
    fixture.finish().await;

    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value = json_body(response).await;
    let api = body["servers"]
        .as_array()
        .and_then(|servers| servers.iter().find(|server| server["name"] == "api"))
        .expect("api MCP server status");
    assert_eq!(api["status"], "needs_auth");
    let rendered = body.to_string();
    assert!(
        rendered.contains("authorization required"),
        "server status remains actionable: {rendered}"
    );
    assert!(
        !rendered.contains(OPAQUE_AUTH_CHALLENGE)
            && !rendered.contains("challenge-must-not-reach-server"),
        "server surface must never serialize OAuth challenges: {rendered}"
    );
}

const OPAQUE_AUTH_CHALLENGE: &str =
    "Bearer realm=\"fixture\", opaque_challenge=\"challenge-must-not-reach-server\"";

/// A self-contained remote-MCP probe fixture. Its sole response is a real HTTP 401 to an
/// initialize POST, and the spawned task has a deadline and is awaited by the test.
struct NeedsAuthFixture {
    url: String,
    handle: Option<tokio::task::JoinHandle<Result<(), String>>>,
}

impl NeedsAuthFixture {
    async fn start() -> Self {
        let listener = TcpListener::bind(("127.0.0.1", 0))
            .await
            .expect("bind ephemeral needs-auth fixture");
        let port = listener
            .local_addr()
            .expect("needs-auth fixture local address")
            .port();
        let handle = tokio::spawn(async move {
            tokio::time::timeout(std::time::Duration::from_secs(2), async {
                let (mut stream, _) = listener
                    .accept()
                    .await
                    .map_err(|err| format!("accept MCP probe: {err}"))?;
                let request = read_http_request(&mut stream)
                    .await
                    .map_err(|err| format!("read MCP probe: {err}"))?;
                assert_initialize_probe(&request)?;
                stream
                    .write_all(unauthorized_response().as_bytes())
                    .await
                    .map_err(|err| format!("write MCP 401 response: {err}"))?;
                stream
                    .flush()
                    .await
                    .map_err(|err| format!("flush MCP 401 response: {err}"))?;
                Ok(())
            })
            .await
            .map_err(|_| "timed out waiting for an MCP probe".to_owned())?
        });
        Self {
            url: format!("http://127.0.0.1:{port}/mcp"),
            handle: Some(handle),
        }
    }

    async fn finish(mut self) {
        self.handle
            .take()
            .expect("needs-auth fixture task handle")
            .await
            .expect("needs-auth fixture task should not panic")
            .expect("needs-auth fixture should serve an MCP initialize probe");
    }
}

impl Drop for NeedsAuthFixture {
    fn drop(&mut self) {
        if let Some(handle) = self.handle.take() {
            handle.abort();
        }
    }
}

async fn read_http_request(stream: &mut TcpStream) -> std::io::Result<Vec<u8>> {
    let mut request = Vec::new();
    let mut chunk = [0_u8; 1024];
    let headers_end = loop {
        let read = stream.read(&mut chunk).await?;
        if read == 0 {
            return Ok(request);
        }
        request.extend_from_slice(&chunk[..read]);
        if let Some(position) = request.windows(4).position(|window| window == b"\r\n\r\n") {
            break position + 4;
        }
    };
    let headers = String::from_utf8_lossy(&request[..headers_end]);
    let content_length = headers
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().ok())
                .flatten()
        })
        .unwrap_or(0);
    let mut remaining = content_length.saturating_sub(request.len() - headers_end);
    while remaining > 0 {
        let read = stream.read(&mut chunk).await?;
        if read == 0 {
            break;
        }
        request.extend_from_slice(&chunk[..read]);
        remaining = remaining.saturating_sub(read);
    }
    Ok(request)
}

fn assert_initialize_probe(request: &[u8]) -> Result<(), String> {
    let request =
        std::str::from_utf8(request).map_err(|err| format!("MCP probe was not UTF-8: {err}"))?;
    let (headers, body) = request
        .split_once("\r\n\r\n")
        .ok_or_else(|| "MCP probe omitted HTTP headers".to_owned())?;
    if !headers
        .lines()
        .next()
        .is_some_and(|line| line.starts_with("POST /mcp "))
    {
        return Err(format!("expected an MCP POST probe, got {headers:?}"));
    }
    let method = serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|body| body["method"].as_str().map(str::to_owned));
    if method.as_deref() != Some("initialize") {
        return Err(format!("expected an MCP initialize probe, got {method:?}"));
    }
    Ok(())
}

fn unauthorized_response() -> String {
    format!(
        "HTTP/1.1 401 Unauthorized\r\nWWW-Authenticate: {OPAQUE_AUTH_CHALLENGE}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
    )
}

struct Scratch {
    dir: PathBuf,
    proc_tag: String,
}

impl Scratch {
    fn new(label: &str) -> Self {
        let id = SCRATCH_COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("cortex-{label}-{}-{id}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("create scratch dir");
        // A unique argv token so a global process scan can identify THIS sandbox's stdio fixture
        // without matching sibling tests' identically-named echo-server binaries. The fixture
        // ignores extra argv, so the token only affects how the process appears in `ps`.
        let proc_tag = format!("cortex-mcp-proc-{}-{id}", std::process::id());
        Self { dir, proc_tag }
    }

    fn db_path(&self) -> PathBuf {
        self.dir.join("cortex.db")
    }

    fn workspace(&self) -> PathBuf {
        let workspace = self.dir.join("workspace");
        std::fs::create_dir_all(&workspace).expect("create workspace dir");
        workspace
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}
