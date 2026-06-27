//! The agent->client side of the ACP stdio connection.
//!
//! The base [`AcpService`](crate::AcpService) answers client requests; this module adds the reverse
//! direction the spec requires for filesystem, terminal, and permission round-trips. A
//! [`ClientConnection`] sends a JSON-RPC request to the client over the same stdout writer, parks a
//! [`oneshot`] keyed by the outbound id, and resolves it when the client's response is routed back
//! through the read loop. The capability-backed tools ([`AcpReadTool`], [`AcpWriteTool`],
//! [`AcpTerminalTool`]) and the [`AcpPermissionResponder`] are materialized for a turn only when the
//! client advertised the matching capability, so a client that offers nothing keeps the local
//! defaults.

use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Condvar, Mutex};

use futures_util::future::BoxFuture;
use permissions::{Decision, PermissionRequest, Responder, ScopedDecision};
use serde_json::{Value, json};
use tokio::sync::oneshot;
use tools::{Tool, ToolContext, ToolOutput};

use crate::{AcpState, JsonRpcWriter};

/// A parked client round-trip awaiting the client's JSON-RPC response.
///
/// Filesystem and terminal tools await asynchronously over a [`oneshot`]; the synchronous
/// permission [`Responder`] seam instead parks a condvar-backed slot so the blocking wait never
/// touches the async runtime (a tokio `blocking_recv` would panic inside a runtime worker). Both are
/// resolved uniformly by the read loop through [`PendingReply::resolve`].
pub(crate) enum PendingReply {
    Async(oneshot::Sender<Result<Value, String>>),
    Sync(Arc<SyncReply>),
}

impl PendingReply {
    pub(crate) fn resolve(self, result: Result<Value, String>) {
        match self {
            PendingReply::Async(tx) => {
                let _ = tx.send(result);
            }
            PendingReply::Sync(slot) => slot.resolve(result),
        }
    }
}

/// A condvar-backed reply slot used by the synchronous permission round-trip.
pub(crate) struct SyncReply {
    slot: Mutex<Option<Result<Value, String>>>,
    ready: Condvar,
}

impl SyncReply {
    fn new() -> Self {
        Self {
            slot: Mutex::new(None),
            ready: Condvar::new(),
        }
    }

    fn wait(&self) -> Result<Value, String> {
        let mut guard = self.slot.lock().expect("sync reply lock");
        loop {
            if let Some(reply) = guard.take() {
                return reply;
            }
            guard = self.ready.wait(guard).expect("sync reply wait");
        }
    }

    fn resolve(&self, result: Result<Value, String>) {
        if let Ok(mut guard) = self.slot.lock()
            && guard.is_none()
        {
            *guard = Some(result);
        }
        self.ready.notify_all();
    }
}

/// A type-erased outbound JSON-RPC sink over the stdout writer.
///
/// The writer is generic over its byte sink, but the tools and responder are trait objects, so the
/// reverse channel is erased behind this trait once the concrete writer type is known (in the
/// prompt-running path).
pub(crate) trait OutboundSink: Send + Sync {
    fn send(&self, value: Value);
}

impl<W: Write + Send> OutboundSink for JsonRpcWriter<W> {
    fn send(&self, value: Value) {
        let _ = self.send_value(value);
    }
}

/// The agent's handle for issuing requests to the connected ACP client and awaiting their replies.
#[derive(Clone)]
pub(crate) struct ClientConnection {
    state: Arc<AcpState>,
    sink: Arc<dyn OutboundSink>,
    session_id: String,
}

impl ClientConnection {
    pub(crate) fn new(
        state: Arc<AcpState>,
        sink: Arc<dyn OutboundSink>,
        session_id: String,
    ) -> Self {
        Self {
            state,
            sink,
            session_id,
        }
    }

    pub(crate) fn session_id(&self) -> &str {
        &self.session_id
    }

    /// Send `method` with `params` to the client and await its JSON-RPC result (or error message).
    pub(crate) async fn request(&self, method: &str, params: Value) -> Result<Value, String> {
        let seq = self.state.client_seq.fetch_add(1, Ordering::Relaxed);
        let id = format!("acp-srv-{seq}");
        let (tx, rx) = oneshot::channel();
        self.park(&id, PendingReply::Async(tx));
        self.emit(&id, method, params);
        match rx.await {
            Ok(result) => result,
            Err(_) => Err(format!(
                "ACP client closed before answering {method} ({id})"
            )),
        }
    }

    /// Send `method` with `params` to the client and block the current thread for its reply.
    ///
    /// The permission [`Responder`] seam is synchronous but the round-trip is async. The reply is
    /// parked on a condvar slot and waited on with [`tokio::task::block_in_place`] when running on a
    /// multi-thread runtime, so the read loop keeps running on another worker and delivers the reply
    /// that unblocks this call (mirrors the server permission responder).
    pub(crate) fn request_blocking(&self, method: &str, params: Value) -> Result<Value, String> {
        let seq = self.state.client_seq.fetch_add(1, Ordering::Relaxed);
        let id = format!("acp-srv-{seq}");
        let slot = Arc::new(SyncReply::new());
        self.park(&id, PendingReply::Sync(slot.clone()));
        self.emit(&id, method, params);
        let wait = move || slot.wait();
        match tokio::runtime::Handle::try_current() {
            Ok(handle) if handle.runtime_flavor() == tokio::runtime::RuntimeFlavor::MultiThread => {
                tokio::task::block_in_place(wait)
            }
            _ => wait(),
        }
    }

    fn park(&self, id: &str, reply: PendingReply) {
        self.state
            .client_pending
            .lock()
            .expect("client pending mutex")
            .insert(id.to_owned(), reply);
    }

    fn emit(&self, id: &str, method: &str, params: Value) {
        self.sink.send(json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        }));
    }
}

fn absolute_path(ctx: &ToolContext, path: &str) -> PathBuf {
    ctx.resolve(path)
}

/// The `read` tool backed by the client's `fs/read_text_file` capability.
pub(crate) struct AcpReadTool {
    connection: ClientConnection,
}

impl AcpReadTool {
    pub(crate) fn new(connection: ClientConnection) -> Self {
        Self { connection }
    }
}

impl Tool for AcpReadTool {
    fn name(&self) -> &str {
        "read"
    }

    fn description(&self) -> &str {
        "Read a UTF-8 text file through the connected client's filesystem service. Provide an \
         absolute or workspace-relative path; optional 1-based offset and line limit."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "File path to read." },
                "offset": { "type": "integer", "description": "1-based line to start from." },
                "limit": { "type": "integer", "description": "Maximum number of lines to read." }
            },
            "required": ["path"],
            "additionalProperties": false
        })
    }

    fn execute<'a>(&'a self, arguments: Value, ctx: &'a ToolContext) -> BoxFuture<'a, ToolOutput> {
        Box::pin(async move {
            let Some(path) = arguments.get("path").and_then(Value::as_str) else {
                return ToolOutput::error("read: missing required `path` argument");
            };
            let resolved = absolute_path(ctx, path);
            let mut params = json!({
                "sessionId": self.connection.session_id(),
                "path": resolved.to_string_lossy(),
            });
            if let Some(offset) = arguments.get("offset").and_then(Value::as_u64) {
                params["line"] = json!(offset);
            }
            if let Some(limit) = arguments.get("limit").and_then(Value::as_u64) {
                params["limit"] = json!(limit);
            }
            match self.connection.request("fs/read_text_file", params).await {
                Ok(value) => {
                    let content = value
                        .get("content")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned();
                    ctx.record_read(resolved);
                    ToolOutput::text(content)
                }
                Err(message) => ToolOutput::error(format!("read: {message}")),
            }
        })
    }
}

/// The `write` tool backed by the client's `fs/write_text_file` capability.
pub(crate) struct AcpWriteTool {
    connection: ClientConnection,
}

impl AcpWriteTool {
    pub(crate) fn new(connection: ClientConnection) -> Self {
        Self { connection }
    }
}

impl Tool for AcpWriteTool {
    fn name(&self) -> &str {
        "write"
    }

    fn description(&self) -> &str {
        "Write UTF-8 text to a file through the connected client's filesystem service, replacing \
         any existing contents. Provide an absolute or workspace-relative path and the full content."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "File path to write." },
                "content": { "type": "string", "description": "Full file content to write." }
            },
            "required": ["path", "content"],
            "additionalProperties": false
        })
    }

    fn execute<'a>(&'a self, arguments: Value, ctx: &'a ToolContext) -> BoxFuture<'a, ToolOutput> {
        Box::pin(async move {
            let Some(path) = arguments.get("path").and_then(Value::as_str) else {
                return ToolOutput::error("write: missing required `path` argument");
            };
            let Some(content) = arguments.get("content").and_then(Value::as_str) else {
                return ToolOutput::error("write: missing required `content` argument");
            };
            let resolved = absolute_path(ctx, path);
            let params = json!({
                "sessionId": self.connection.session_id(),
                "path": resolved.to_string_lossy(),
                "content": content,
            });
            match self.connection.request("fs/write_text_file", params).await {
                Ok(_) => ToolOutput::text(format!(
                    "wrote {} bytes to {}",
                    content.len(),
                    resolved.to_string_lossy()
                )),
                Err(message) => ToolOutput::error(format!("write: {message}")),
            }
        })
    }
}

/// The `bash` tool backed by the client's `terminal/*` capability.
pub(crate) struct AcpTerminalTool {
    connection: ClientConnection,
}

impl AcpTerminalTool {
    pub(crate) fn new(connection: ClientConnection) -> Self {
        Self { connection }
    }
}

impl Tool for AcpTerminalTool {
    fn name(&self) -> &str {
        "bash"
    }

    fn description(&self) -> &str {
        "Run a shell command through the connected client's terminal service and return its output \
         and exit status. The client owns the process lifecycle; the terminal is always released."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "command": { "type": "string", "description": "The shell command to run." }
            },
            "required": ["command"],
            "additionalProperties": false
        })
    }

    fn execute<'a>(&'a self, arguments: Value, _ctx: &'a ToolContext) -> BoxFuture<'a, ToolOutput> {
        Box::pin(async move {
            let Some(command) = arguments.get("command").and_then(Value::as_str) else {
                return ToolOutput::error("bash: missing required `command` argument");
            };
            let session_id = self.connection.session_id().to_owned();
            let create = json!({
                "sessionId": session_id,
                "command": "bash",
                "args": ["-c", command],
            });
            let terminal_id = match self.connection.request("terminal/create", create).await {
                Ok(value) => match value.get("terminalId").and_then(Value::as_str) {
                    Some(id) => id.to_owned(),
                    None => {
                        return ToolOutput::error(
                            "bash: client terminal/create returned no terminalId",
                        );
                    }
                },
                Err(message) => return ToolOutput::error(format!("bash: {message}")),
            };

            let wait_params = json!({ "sessionId": session_id, "terminalId": terminal_id });
            let exit = self
                .connection
                .request("terminal/wait_for_exit", wait_params)
                .await;

            let output_params = json!({ "sessionId": session_id, "terminalId": terminal_id });
            let output_value = self
                .connection
                .request("terminal/output", output_params)
                .await;

            let release_params = json!({ "sessionId": session_id, "terminalId": terminal_id });
            let _ = self
                .connection
                .request("terminal/release", release_params)
                .await;

            let output = output_value
                .as_ref()
                .ok()
                .and_then(|value| value.get("output").and_then(Value::as_str))
                .unwrap_or_default()
                .to_owned();
            let exit_code = match &exit {
                Ok(value) => value.get("exitCode").and_then(Value::as_i64),
                Err(_) => None,
            };
            let mut text = output;
            match exit_code {
                Some(code) => text.push_str(&format!("\n[exit code: {code}]")),
                None => text.push_str("\n[exit code: unknown]"),
            }
            if let Err(message) = exit {
                return ToolOutput::error(format!("bash: {message}\n{text}"));
            }
            ToolOutput::text(text)
        })
    }
}

/// Maps a tool name to the ACP [`ToolKind`](agent_client_protocol::schema::v1::ToolKind) wire value.
pub(crate) fn tool_kind(name: &str) -> &'static str {
    match name {
        "read" | "glob" | "grep" | "lsp" => "read",
        "write" | "edit" | "apply_patch" => "edit",
        "bash" => "execute",
        "webfetch" | "websearch" => "fetch",
        _ => "other",
    }
}

/// The permission [`Responder`] that asks the connected ACP client to resolve a gated action.
pub(crate) struct AcpPermissionResponder {
    connection: ClientConnection,
}

impl AcpPermissionResponder {
    pub(crate) fn new(connection: ClientConnection) -> Self {
        Self { connection }
    }
}

impl Responder for AcpPermissionResponder {
    fn respond(&self, request: &PermissionRequest) -> Decision {
        self.respond_scoped(request).decision
    }

    fn respond_scoped(&self, request: &PermissionRequest) -> ScopedDecision {
        let seq = self.connection.state.client_seq.load(Ordering::Relaxed);
        let tool_call_id = format!("perm-{seq}");
        let params = json!({
            "sessionId": self.connection.session_id(),
            "toolCall": {
                "toolCallId": tool_call_id,
                "title": format!("{}: {}", request.tool, request.subject),
                "kind": tool_kind(&request.tool),
                "status": "pending",
                "rawInput": {
                    "tool": request.tool,
                    "subject": request.subject,
                },
                "locations": [ { "path": request.subject } ],
            },
            "options": [
                { "optionId": "allow-once", "name": "Allow once", "kind": "allow_once" },
                { "optionId": "allow-always", "name": "Allow always", "kind": "allow_always" },
                { "optionId": "reject-once", "name": "Reject once", "kind": "reject_once" },
                { "optionId": "reject-always", "name": "Reject always", "kind": "reject_always" },
            ],
        });
        match self
            .connection
            .request_blocking("session/request_permission", params)
        {
            Ok(value) => decision_from_outcome(&value),
            // Fail closed: a client that does not answer must not let the action proceed.
            Err(_) => ScopedDecision::once(Decision::Deny),
        }
    }
}

fn decision_from_outcome(value: &Value) -> ScopedDecision {
    let outcome = value.get("outcome");
    let feedback = outcome
        .and_then(|outcome| outcome.get("_meta"))
        .and_then(|meta| meta.get("feedback"))
        .and_then(Value::as_str)
        .map(str::to_owned);
    let kind = outcome
        .and_then(|outcome| outcome.get("outcome"))
        .and_then(Value::as_str)
        .unwrap_or("cancelled");
    if kind != "selected" {
        return ScopedDecision::once(Decision::Deny);
    }
    let option = outcome
        .and_then(|outcome| outcome.get("optionId"))
        .and_then(Value::as_str)
        .unwrap_or("reject-once");
    let mut decision = match option {
        "allow-once" => ScopedDecision::once(Decision::Allow),
        "allow-always" => ScopedDecision::always(Decision::Allow),
        "reject-always" => ScopedDecision::always(Decision::Deny),
        _ => ScopedDecision::once(Decision::Deny),
    };
    if let Some(feedback) = feedback {
        decision = decision.with_feedback(feedback);
    }
    decision
}
