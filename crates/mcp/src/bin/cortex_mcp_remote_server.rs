//! A real remote MCP server fixture used as black-box proof for the remote MCP client.
//!
//! It speaks the Model Context Protocol over HTTP and runs in one of three modes (`CORTEX_MCP_MODE`):
//!
//! - `streamable` (default): the modern Streamable HTTP transport. `POST <path>` answers JSON-RPC
//!   with `application/json` (initialize returns an `Mcp-Session-Id`); `GET <path>` opens a
//!   server→client SSE notification stream. Proves VAL-MCP-006.
//! - `sse`: the legacy HTTP+SSE transport. `POST` to the MCP path is rejected with `405` (no
//!   Streamable HTTP), while `GET /sse` opens an SSE stream that first emits an `endpoint` event
//!   naming the message-POST URL. Proves the client's SSE fallback, VAL-MCP-007.
//! - `oauth`: like `streamable` but every MCP request requires `Authorization: Bearer <token>`;
//!   without one it answers `401` + `WWW-Authenticate`. It exposes `/authorize` (redirects back to
//!   the loopback callback with a code), `/token` (code→token and refresh_token→token), and OAuth
//!   metadata. Proves VAL-MCP-009/010.
//!
//! Control endpoints mutate live MCP state and emit notifications to connected clients:
//! `/control/add-tool|remove-tool`, `/control/add-resource|remove-resource`,
//! `/control/add-prompt|remove-prompt`, and `/control/log-message`. This lets black-box tests prove
//! live catalogue refresh and log-notification bridging without reconnecting.
//!
//! This is a test/proof fixture, not part of the shipped `cortex` binary. The OAuth token value is
//! never written to stderr or the log file.

use std::collections::{HashMap, HashSet};
use std::fs::OpenOptions;
use std::io::Write as _;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{Mutex, mpsc};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Streamable,
    Sse,
    Oauth,
}

struct State {
    mode: Mode,
    label: String,
    log: Option<String>,
    extra_enabled: AtomicBool,
    resource_enabled: AtomicBool,
    prompt_enabled: AtomicBool,
    streams: Mutex<Vec<mpsc::UnboundedSender<String>>>,
    issued_access: Mutex<HashSet<String>>,
    issued_refresh: Mutex<HashSet<String>>,
    valid_codes: Mutex<HashSet<String>>,
    token_seq: AtomicU64,
    expires_in: u64,
    required_authorization: Option<String>,
}

impl State {
    fn log(&self, line: &str) {
        eprintln!("{line}");
        if let Some(path) = &self.log {
            if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
                let _ = writeln!(file, "{line}");
            }
        }
    }

    /// The advertised tools as JSON-RPC `tools/list` tool objects.
    fn tool_list(&self) -> Vec<Value> {
        let mut tools = vec![
            json!({
                "name": "add",
                "description": "Add two integers a and b and return their sum.",
                "inputSchema": {
                    "type": "object",
                    "properties": { "a": {"type":"integer"}, "b": {"type":"integer"} },
                    "required": ["a","b"],
                    "additionalProperties": false
                }
            }),
            json!({
                "name": "echo",
                "description": "Echo back the provided message.",
                "inputSchema": {
                    "type": "object",
                    "properties": { "message": {"type":"string"} },
                    "required": ["message"],
                    "additionalProperties": false
                }
            }),
            json!({
                "name": "search",
                "description": "Search for a query (returns a labelled stub result).",
                "inputSchema": {
                    "type": "object",
                    "properties": { "query": {"type":"string"} },
                    "required": ["query"],
                    "additionalProperties": false
                }
            }),
        ];
        if self.extra_enabled.load(Ordering::SeqCst) {
            tools.push(json!({
                "name": "extra",
                "description": "A tool added at runtime to prove live tool-list updates.",
                "inputSchema": { "type": "object", "properties": {}, "additionalProperties": false }
            }));
        }
        tools
    }

    fn resources(&self) -> Vec<Value> {
        if !self.resource_enabled.load(Ordering::SeqCst) {
            return Vec::new();
        }
        vec![json!({
            "uri": "notes://demo",
            "name": "demo-notes",
            "description": "Remote resource fixture",
            "mimeType": "text/plain"
        })]
    }

    fn prompts(&self) -> Vec<Value> {
        if !self.prompt_enabled.load(Ordering::SeqCst) {
            return Vec::new();
        }
        vec![json!({
            "name": "greet",
            "description": "Remote prompt fixture",
            "arguments": [{ "name": "name", "required": true }]
        })]
    }

    /// Push a notification frame to every open notification stream.
    async fn notify(&self, method: &str, params: Option<Value>) {
        let mut payload = json!({"jsonrpc":"2.0","method":method});
        if let Some(params) = params {
            payload["params"] = params;
        }
        let frame = format!("event: message\ndata: {payload}\n\n");
        let mut streams = self.streams.lock().await;
        streams.retain(|tx| tx.send(frame.clone()).is_ok());
    }

    async fn notify_tools_changed(&self) {
        self.notify("notifications/tools/list_changed", None).await;
    }

    async fn notify_resources_changed(&self) {
        self.notify("notifications/resources/list_changed", None)
            .await;
    }

    async fn notify_prompts_changed(&self) {
        self.notify("notifications/prompts/list_changed", None)
            .await;
    }

    async fn register_stream(&self, tx: mpsc::UnboundedSender<String>) {
        self.streams.lock().await.push(tx);
    }

    fn issue_tokens(&self) -> (String, String, Value) {
        let n = self.token_seq.fetch_add(1, Ordering::SeqCst);
        let access = format!("cortex-access-{}-{}", n, rand_hex());
        let refresh = format!("cortex-refresh-{}-{}", n, rand_hex());
        let body = json!({
            "access_token": access,
            "refresh_token": refresh,
            "token_type": "Bearer",
            "expires_in": self.expires_in,
        });
        (access, refresh, body)
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mode = match std::env::var("CORTEX_MCP_MODE").as_deref() {
        Ok("sse") => Mode::Sse,
        Ok("oauth") => Mode::Oauth,
        _ => Mode::Streamable,
    };
    let port: u16 = std::env::var("CORTEX_MCP_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .expect("CORTEX_MCP_PORT must be set to a port in 3200-3299");
    let expires_in: u64 = std::env::var("CORTEX_MCP_EXPIRES_IN")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(3600);
    let required_authorization = std::env::var("CORTEX_MCP_REQUIRE_AUTHORIZATION")
        .ok()
        .filter(|value| !value.trim().is_empty());

    let state = Arc::new(State {
        mode,
        label: std::env::var("CORTEX_MCP_LABEL").unwrap_or_else(|_| "remote".to_owned()),
        log: std::env::var("CORTEX_MCP_LOG").ok(),
        extra_enabled: AtomicBool::new(false),
        resource_enabled: AtomicBool::new(false),
        prompt_enabled: AtomicBool::new(false),
        streams: Mutex::new(Vec::new()),
        issued_access: Mutex::new(HashSet::new()),
        issued_refresh: Mutex::new(HashSet::new()),
        valid_codes: Mutex::new(HashSet::new()),
        token_seq: AtomicU64::new(1),
        expires_in,
        required_authorization,
    });

    let listener = TcpListener::bind(("127.0.0.1", port)).await?;
    state.log(&format!(
        "cortex-mcp-remote-server listening on 127.0.0.1:{port} mode={}",
        mode_name(mode)
    ));

    loop {
        let (stream, _addr) = listener.accept().await?;
        let state = state.clone();
        tokio::spawn(async move {
            if let Err(err) = handle(stream, state).await {
                let _ = err;
            }
        });
    }
}

const fn mode_name(mode: Mode) -> &'static str {
    match mode {
        Mode::Streamable => "streamable",
        Mode::Sse => "sse",
        Mode::Oauth => "oauth",
    }
}

struct Request {
    method: String,
    path: String,
    query: String,
    headers: HashMap<String, String>,
    body: Vec<u8>,
}

async fn handle(mut stream: TcpStream, state: Arc<State>) -> std::io::Result<()> {
    let request = match read_request(&mut stream).await? {
        Some(request) => request,
        None => return Ok(()),
    };

    match (state.mode, request.method.as_str(), request.path.as_str()) {
        // --- OAuth endpoints ---
        (Mode::Oauth, "GET", "/.well-known/oauth-authorization-server") => {
            let host = request
                .headers
                .get("host")
                .cloned()
                .unwrap_or_else(|| "127.0.0.1".to_owned());
            write_json(
                &mut stream,
                200,
                &json!({
                    "issuer": format!("http://{host}"),
                    "authorization_endpoint": format!("http://{host}/authorize"),
                    "token_endpoint": format!("http://{host}/token"),
                    "response_types_supported": ["code"],
                    "grant_types_supported": ["authorization_code", "refresh_token"],
                }),
            )
            .await
        }
        (Mode::Oauth, "GET", "/authorize") => {
            handle_authorize(&mut stream, &state, &request.query).await
        }
        (Mode::Oauth, "POST", "/token") => handle_token(&mut stream, &state, &request.body).await,

        // --- Control endpoints (all modes) ---
        (_, "POST", "/control/add-tool") => {
            state.extra_enabled.store(true, Ordering::SeqCst);
            state.notify_tools_changed().await;
            state.log("control: added tool `extra` and notified tools/list_changed");
            write_text(&mut stream, 200, "added").await
        }
        (_, "POST", "/control/remove-tool") => {
            state.extra_enabled.store(false, Ordering::SeqCst);
            state.notify_tools_changed().await;
            state.log("control: removed tool `extra` and notified tools/list_changed");
            write_text(&mut stream, 200, "removed").await
        }
        (_, "POST", "/control/add-resource") => {
            state.resource_enabled.store(true, Ordering::SeqCst);
            state.notify_resources_changed().await;
            state.log("control: added resource and notified resources/list_changed");
            write_text(&mut stream, 200, "added").await
        }
        (_, "POST", "/control/remove-resource") => {
            state.resource_enabled.store(false, Ordering::SeqCst);
            state.notify_resources_changed().await;
            state.log("control: removed resource and notified resources/list_changed");
            write_text(&mut stream, 200, "removed").await
        }
        (_, "POST", "/control/add-prompt") => {
            state.prompt_enabled.store(true, Ordering::SeqCst);
            state.notify_prompts_changed().await;
            state.log("control: added prompt and notified prompts/list_changed");
            write_text(&mut stream, 200, "added").await
        }
        (_, "POST", "/control/remove-prompt") => {
            state.prompt_enabled.store(false, Ordering::SeqCst);
            state.notify_prompts_changed().await;
            state.log("control: removed prompt and notified prompts/list_changed");
            write_text(&mut stream, 200, "removed").await
        }
        (_, "POST", "/control/log-message") => {
            state
                .notify(
                    "notifications/message",
                    Some(json!({
                        "level": "warning",
                        "logger": "fixture.remote",
                        "data": {
                            "message": "fixture warning",
                            "token": format!("sk-{}-{}", "or-v1", "deadbeef".repeat(8))
                        }
                    })),
                )
                .await;
            state.log("control: emitted notifications/message");
            write_text(&mut stream, 200, "emitted").await
        }

        // --- Legacy SSE transport ---
        (Mode::Sse, "GET", "/sse") => handle_sse_stream(stream, &state).await,
        (Mode::Sse, "POST", path) if path.starts_with("/sse/message") => {
            handle_sse_message(&mut stream, &state, &request.body).await
        }
        // The legacy server rejects a Streamable HTTP POST so the client falls back to SSE.
        (Mode::Sse, "POST", "/sse") => write_text(&mut stream, 405, "Method Not Allowed").await,

        // --- Streamable HTTP transport (and OAuth-gated streamable) ---
        (Mode::Streamable, "POST", _) | (Mode::Oauth, "POST", _) => {
            if !authorized(&state, &request).await {
                return write_unauthorized(&mut stream).await;
            }
            handle_streamable_post(&mut stream, &state, &request.body).await
        }
        (Mode::Streamable, "GET", _) | (Mode::Oauth, "GET", _) => {
            if !authorized(&state, &request).await {
                return write_unauthorized(&mut stream).await;
            }
            state.log("GET SSE notification stream opened");
            handle_notification_stream(stream, &state).await
        }

        _ => write_text(&mut stream, 404, "Not Found").await,
    }
}

/// Whether the request is authorized for the configured mode.
async fn authorized(state: &State, request: &Request) -> bool {
    if let Some(expected) = &state.required_authorization {
        return request
            .headers
            .get("authorization")
            .is_some_and(|actual| actual == expected);
    }
    if state.mode != Mode::Oauth {
        return true;
    }
    let Some(auth) = request.headers.get("authorization") else {
        return false;
    };
    let Some(token) = auth
        .strip_prefix("Bearer ")
        .or_else(|| auth.strip_prefix("bearer "))
    else {
        return false;
    };
    state.issued_access.lock().await.contains(token.trim())
}

async fn write_unauthorized(stream: &mut TcpStream) -> std::io::Result<()> {
    let body = "{\"error\":\"unauthorized\"}";
    let response = format!(
        "HTTP/1.1 401 Unauthorized\r\nContent-Type: application/json\r\nWWW-Authenticate: Bearer realm=\"cortex-mcp\"\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(response.as_bytes()).await?;
    stream.flush().await
}

/// Handle one Streamable HTTP POST: a JSON-RPC request (answered with JSON) or notification (202).
async fn handle_streamable_post(
    stream: &mut TcpStream,
    state: &State,
    body: &[u8],
) -> std::io::Result<()> {
    let request: Value = match serde_json::from_slice(body) {
        Ok(value) => value,
        Err(_) => return write_text(stream, 400, "invalid JSON").await,
    };
    match dispatch(state, &request).await {
        Some(response) => {
            let session = "cortex-remote-session";
            write_json_with_session(stream, 200, &response, session).await
        }
        None => write_accepted(stream).await,
    }
}

/// Dispatch a JSON-RPC request, returning the response JSON for a request id, or `None` for a
/// notification.
async fn dispatch(state: &State, request: &Value) -> Option<Value> {
    let method = request
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let id = request.get("id").cloned();

    match method {
        "initialize" => {
            state.log(&format!(
                "HTTP MCP session established ({})",
                mode_name(state.mode)
            ));
            Some(json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": {
                    "protocolVersion": "2025-03-26",
                    "capabilities": {
                        "logging": {},
                        "tools": { "listChanged": true },
                        "resources": { "listChanged": true },
                        "prompts": { "listChanged": true }
                    },
                    "serverInfo": { "name": "cortex-mcp-remote-fixture", "version": "0.1.0" }
                }
            }))
        }
        "notifications/initialized" | "notifications/cancelled" => None,
        "ping" => Some(json!({"jsonrpc":"2.0","id":id,"result":{}})),
        "tools/list" => Some(json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": { "tools": state.tool_list() }
        })),
        "resources/list" => Some(json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": { "resources": state.resources() }
        })),
        "prompts/list" => Some(json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": { "prompts": state.prompts() }
        })),
        "tools/call" => {
            let params = request.get("params").cloned().unwrap_or(Value::Null);
            let name = params
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let arguments = params.get("arguments").cloned().unwrap_or(Value::Null);
            state.log(&format!("tools/call name={name} args={arguments}"));
            Some(
                json!({ "jsonrpc": "2.0", "id": id, "result": tool_result(state, name, &arguments) }),
            )
        }
        _ => Some(json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": { "code": -32601, "message": format!("unknown method: {method}") }
        })),
    }
}

/// Build the `tools/call` result for a tool.
fn tool_result(state: &State, name: &str, arguments: &Value) -> Value {
    match name {
        "add" => {
            let a = arguments.get("a").and_then(Value::as_i64).unwrap_or(0);
            let b = arguments.get("b").and_then(Value::as_i64).unwrap_or(0);
            let sum = a + b;
            json!({
                "content": [{ "type": "text", "text": sum.to_string() }],
                "structuredContent": { "sum": sum },
                "isError": false
            })
        }
        "echo" => {
            let message = arguments
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("");
            json!({
                "content": [{ "type": "text", "text": message }],
                "structuredContent": { "echo": message },
                "isError": false
            })
        }
        "search" => {
            let query = arguments.get("query").and_then(Value::as_str).unwrap_or("");
            let text = format!("{}: results for {query}", state.label);
            json!({
                "content": [{ "type": "text", "text": text }],
                "isError": false
            })
        }
        "extra" => json!({
            "content": [{ "type": "text", "text": "extra tool result" }],
            "isError": false
        }),
        other => json!({
            "content": [{ "type": "text", "text": format!("unknown tool: {other}") }],
            "isError": true
        }),
    }
}

/// Stream server→client notifications over an SSE GET (Streamable HTTP).
async fn handle_notification_stream(mut stream: TcpStream, state: &State) -> std::io::Result<()> {
    let (tx, mut rx) = mpsc::unbounded_channel::<String>();
    state.register_stream(tx).await;
    let header = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nConnection: keep-alive\r\n\r\n";
    stream.write_all(header.as_bytes()).await?;
    stream.write_all(b": connected\n\n").await?;
    stream.flush().await?;
    while let Some(frame) = rx.recv().await {
        if stream.write_all(frame.as_bytes()).await.is_err() {
            break;
        }
        let _ = stream.flush().await;
    }
    Ok(())
}

/// Open the legacy SSE stream: emit the `endpoint` event, then stream notifications/responses.
async fn handle_sse_stream(mut stream: TcpStream, state: &State) -> std::io::Result<()> {
    let (tx, mut rx) = mpsc::unbounded_channel::<String>();
    state.register_stream(tx).await;
    let header = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nConnection: keep-alive\r\n\r\n";
    stream.write_all(header.as_bytes()).await?;
    let endpoint = "/sse/message?session=cortex-remote-session";
    state.log(&format!("SSE endpoint opened: {endpoint}"));
    stream
        .write_all(format!("event: endpoint\ndata: {endpoint}\n\n").as_bytes())
        .await?;
    stream.flush().await?;
    while let Some(frame) = rx.recv().await {
        if stream.write_all(frame.as_bytes()).await.is_err() {
            break;
        }
        let _ = stream.flush().await;
    }
    Ok(())
}

/// Handle a legacy SSE message POST: dispatch and deliver the response over the SSE stream.
async fn handle_sse_message(
    stream: &mut TcpStream,
    state: &State,
    body: &[u8],
) -> std::io::Result<()> {
    let request: Value = match serde_json::from_slice(body) {
        Ok(value) => value,
        Err(_) => return write_text(stream, 400, "invalid JSON").await,
    };
    if let Some(response) = dispatch(state, &request).await {
        let frame = format!("event: message\ndata: {response}\n\n");
        let mut streams = state.streams.lock().await;
        streams.retain(|tx| tx.send(frame.clone()).is_ok());
    }
    write_accepted(stream).await
}

/// Handle the OAuth `/authorize` request: validate and redirect to the loopback callback with a code.
async fn handle_authorize(
    stream: &mut TcpStream,
    state: &State,
    query: &str,
) -> std::io::Result<()> {
    let params: HashMap<String, String> = url::form_urlencoded::parse(query.as_bytes())
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();
    let Some(redirect_uri) = params.get("redirect_uri") else {
        return write_text(stream, 400, "missing redirect_uri").await;
    };
    let state_param = params.get("state").cloned().unwrap_or_default();
    if params
        .get("client_id")
        .map(String::as_str)
        .unwrap_or_default()
        .is_empty()
    {
        return write_text(stream, 400, "missing client_id").await;
    }

    let code = format!("code-{}", rand_hex());
    state.valid_codes.lock().await.insert(code.clone());
    state.log("authorize: issued code and redirecting to loopback callback");

    let separator = if redirect_uri.contains('?') { '&' } else { '?' };
    let location = format!(
        "{redirect_uri}{separator}code={}&state={}",
        urlencode(&code),
        urlencode(&state_param)
    );
    let response = format!(
        "HTTP/1.1 302 Found\r\nLocation: {location}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
    );
    stream.write_all(response.as_bytes()).await?;
    stream.flush().await
}

/// Handle the OAuth `/token` request: authorization_code → tokens, refresh_token → fresh tokens.
async fn handle_token(stream: &mut TcpStream, state: &State, body: &[u8]) -> std::io::Result<()> {
    let params: HashMap<String, String> = url::form_urlencoded::parse(body)
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();
    let grant = params
        .get("grant_type")
        .map(String::as_str)
        .unwrap_or_default();

    match grant {
        "authorization_code" => {
            let code = params.get("code").map(String::as_str).unwrap_or_default();
            if !state.valid_codes.lock().await.remove(code) {
                return write_json(stream, 400, &json!({"error":"invalid_grant"})).await;
            }
            let (access, refresh, body) = state.issue_tokens();
            state.issued_access.lock().await.insert(access);
            state.issued_refresh.lock().await.insert(refresh);
            state.log("token: authorization_code grant -> issued tokens");
            write_json(stream, 200, &body).await
        }
        "refresh_token" => {
            let refresh = params
                .get("refresh_token")
                .map(String::as_str)
                .unwrap_or_default();
            if !state.issued_refresh.lock().await.contains(refresh) {
                return write_json(stream, 400, &json!({"error":"invalid_grant"})).await;
            }
            let (access, new_refresh, body) = state.issue_tokens();
            state.issued_access.lock().await.insert(access);
            state.issued_refresh.lock().await.insert(new_refresh);
            state.log("token: refresh_token grant -> issued fresh access token");
            write_json(stream, 200, &body).await
        }
        _ => write_json(stream, 400, &json!({"error":"unsupported_grant_type"})).await,
    }
}

// --- HTTP helpers ---

async fn read_request(stream: &mut TcpStream) -> std::io::Result<Option<Request>> {
    let mut buffer = Vec::with_capacity(2048);
    let mut chunk = [0u8; 1024];
    let header_end = loop {
        if let Some(pos) = find_subsequence(&buffer, b"\r\n\r\n") {
            break pos;
        }
        let read = stream.read(&mut chunk).await?;
        if read == 0 {
            return Ok(None);
        }
        buffer.extend_from_slice(&chunk[..read]);
        if buffer.len() > 1_048_576 {
            return Ok(None);
        }
    };

    let header_text = String::from_utf8_lossy(&buffer[..header_end]).into_owned();
    let mut lines = header_text.split("\r\n");
    let request_line = lines.next().unwrap_or_default();
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or_default().to_owned();
    let target = parts.next().unwrap_or_default().to_owned();
    let (path, query) = match target.split_once('?') {
        Some((p, q)) => (p.to_owned(), q.to_owned()),
        None => (target, String::new()),
    };

    let mut headers = HashMap::new();
    for line in lines {
        if let Some((name, value)) = line.split_once(':') {
            headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_owned());
        }
    }

    let content_length: usize = headers
        .get("content-length")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let mut body = buffer[header_end + 4..].to_vec();
    while body.len() < content_length {
        let read = stream.read(&mut chunk).await?;
        if read == 0 {
            break;
        }
        body.extend_from_slice(&chunk[..read]);
    }
    body.truncate(content_length);

    Ok(Some(Request {
        method,
        path,
        query,
        headers,
        body,
    }))
}

async fn write_json(stream: &mut TcpStream, status: u16, value: &Value) -> std::io::Result<()> {
    let body = value.to_string();
    let response = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len(),
        reason = reason(status),
    );
    stream.write_all(response.as_bytes()).await?;
    stream.flush().await
}

async fn write_json_with_session(
    stream: &mut TcpStream,
    status: u16,
    value: &Value,
    session: &str,
) -> std::io::Result<()> {
    let body = value.to_string();
    let response = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nMcp-Session-Id: {session}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len(),
        reason = reason(status),
    );
    stream.write_all(response.as_bytes()).await?;
    stream.flush().await
}

async fn write_text(stream: &mut TcpStream, status: u16, message: &str) -> std::io::Result<()> {
    let response = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{message}",
        message.len(),
        reason = reason(status),
    );
    stream.write_all(response.as_bytes()).await?;
    stream.flush().await
}

async fn write_accepted(stream: &mut TcpStream) -> std::io::Result<()> {
    let response = "HTTP/1.1 202 Accepted\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
    stream.write_all(response.as_bytes()).await?;
    stream.flush().await
}

const fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        202 => "Accepted",
        302 => "Found",
        400 => "Bad Request",
        401 => "Unauthorized",
        404 => "Not Found",
        405 => "Method Not Allowed",
        _ => "OK",
    }
}

fn find_subsequence(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn urlencode(input: &str) -> String {
    url::form_urlencoded::byte_serialize(input.as_bytes()).collect()
}

fn rand_hex() -> String {
    let mut bytes = [0u8; 8];
    if let Ok(mut file) = std::fs::File::open("/dev/urandom") {
        use std::io::Read as _;
        let _ = file.read_exact(&mut bytes);
    } else {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or_default();
        bytes.copy_from_slice(&(nanos as u64).to_le_bytes());
    }
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
