//! A real stdio MCP server used as black-box proof for the MCP client.
//!
//! It speaks the Model Context Protocol over stdio (via `rmcp`) and advertises a small set of real
//! tools: `add` (sum two integers), `echo` (echo a message), `search` (a labelled stub used to prove
//! cross-server id namespacing), `big` (emit an oversized payload to prove output truncation), and
//! `env` (report the child's working directory and a named environment variable, used to prove the
//! configured `cwd`/`environment` are applied and that host secrets do not leak into the child).
//! Every `tools/call` it receives is logged with its arguments to stderr and, when
//! `CORTEX_MCP_ECHO_LOG` is set, appended to that file, so a black-box validator can confirm the real
//! call reached the server. An optional `CORTEX_MCP_ECHO_LABEL` distinguishes two instances of this
//! same binary spawned under different server names.
//!
//! This is a test/proof fixture, not part of the shipped `cortex` binary.

use std::borrow::Cow;
use std::fs::OpenOptions;
use std::io::Write as _;
use std::sync::Arc;

use rmcp::ErrorData as McpError;
use rmcp::ServiceExt;
use rmcp::handler::server::ServerHandler;
use rmcp::model::{
    CallToolRequestParams, CallToolResult, Content, JsonObject, ListToolsResult,
    PaginatedRequestParams, ServerCapabilities, ServerInfo, Tool,
};
use rmcp::service::{RequestContext, RoleServer};
use serde_json::{Value, json};

#[derive(Clone)]
struct EchoServer {
    label: Option<String>,
}

impl EchoServer {
    fn new() -> Self {
        Self {
            label: std::env::var("CORTEX_MCP_ECHO_LABEL").ok(),
        }
    }

    fn schema(value: Value) -> Arc<JsonObject> {
        let object = match value {
            Value::Object(map) => map,
            _ => JsonObject::new(),
        };
        Arc::new(object)
    }

    fn tools(&self) -> Vec<Tool> {
        vec![
            Tool::new(
                Cow::Borrowed("add"),
                Cow::Borrowed("Add two integers a and b and return their sum."),
                Self::schema(json!({
                    "type": "object",
                    "properties": {
                        "a": { "type": "integer" },
                        "b": { "type": "integer" }
                    },
                    "required": ["a", "b"],
                    "additionalProperties": false
                })),
            ),
            Tool::new(
                Cow::Borrowed("echo"),
                Cow::Borrowed("Echo back the provided message."),
                Self::schema(json!({
                    "type": "object",
                    "properties": { "message": { "type": "string" } },
                    "required": ["message"],
                    "additionalProperties": false
                })),
            ),
            Tool::new(
                Cow::Borrowed("search"),
                Cow::Borrowed("Search for a query (returns a labelled stub result)."),
                Self::schema(json!({
                    "type": "object",
                    "properties": { "query": { "type": "string" } },
                    "required": ["query"],
                    "additionalProperties": false
                })),
            ),
            Tool::new(
                Cow::Borrowed("big"),
                Cow::Borrowed("Return a payload of the requested size in bytes."),
                Self::schema(json!({
                    "type": "object",
                    "properties": { "size": { "type": "integer" } },
                    "required": ["size"],
                    "additionalProperties": false
                })),
            ),
            Tool::new(
                Cow::Borrowed("env"),
                Cow::Borrowed(
                    "Report the server child's working directory and the value of a named \
                     environment variable.",
                ),
                Self::schema(json!({
                    "type": "object",
                    "properties": { "name": { "type": "string" } },
                    "additionalProperties": false
                })),
            ),
        ]
    }

    fn log_call(&self, name: &str, arguments: &Option<JsonObject>) {
        let args = arguments
            .as_ref()
            .map(|map| Value::Object(map.clone()))
            .unwrap_or(Value::Null);
        let line = format!("tools/call name={name} args={args}");
        eprintln!("{line}");
        if let Ok(path) = std::env::var("CORTEX_MCP_ECHO_LOG") {
            if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
                let _ = writeln!(file, "{line}");
            }
        }
    }

    fn arg_i64(arguments: &Option<JsonObject>, key: &str) -> Result<i64, McpError> {
        arguments
            .as_ref()
            .and_then(|map| map.get(key))
            .and_then(Value::as_i64)
            .ok_or_else(|| {
                McpError::invalid_params(format!("missing integer argument `{key}`"), None)
            })
    }

    fn arg_str(arguments: &Option<JsonObject>, key: &str) -> Result<String, McpError> {
        arguments
            .as_ref()
            .and_then(|map| map.get(key))
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| {
                McpError::invalid_params(format!("missing string argument `{key}`"), None)
            })
    }
}

impl ServerHandler for EchoServer {
    fn get_info(&self) -> ServerInfo {
        let mut info = ServerInfo::default();
        info.instructions = Some("Cortex MCP echo/add proof server.".to_owned());
        info.capabilities = ServerCapabilities::builder().enable_tools().build();
        info
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, McpError> {
        Ok(ListToolsResult {
            tools: self.tools(),
            next_cursor: None,
            meta: None,
        })
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        self.log_call(request.name.as_ref(), &request.arguments);
        match request.name.as_ref() {
            "add" => {
                let a = Self::arg_i64(&request.arguments, "a")?;
                let b = Self::arg_i64(&request.arguments, "b")?;
                let sum = a + b;
                let mut result = CallToolResult::success(vec![Content::text(sum.to_string())]);
                result.structured_content = Some(json!({ "sum": sum }));
                Ok(result)
            }
            "echo" => {
                let message = Self::arg_str(&request.arguments, "message")?;
                let mut result = CallToolResult::success(vec![Content::text(message.clone())]);
                result.structured_content = Some(json!({ "echo": message }));
                Ok(result)
            }
            "search" => {
                let query = Self::arg_str(&request.arguments, "query")?;
                let label = self.label.clone().unwrap_or_else(|| "default".to_owned());
                let text = format!("{label}: results for {query}");
                let mut result = CallToolResult::success(vec![Content::text(text)]);
                result.structured_content = Some(json!({ "label": label, "query": query }));
                Ok(result)
            }
            "big" => {
                let size = Self::arg_i64(&request.arguments, "size")?.max(0) as usize;
                Ok(CallToolResult::success(vec![Content::text(
                    "x".repeat(size),
                )]))
            }
            "env" => {
                // The working directory the child was actually spawned in (proves the configured
                // `cwd` is applied) and the value of an optional named env var (proves the
                // configured `environment` is applied while host secrets like the OpenRouter key
                // are stripped, so querying that name reports it absent).
                let cwd = std::env::current_dir()
                    .map(|path| path.display().to_string())
                    .unwrap_or_default();
                let name = Self::arg_str(&request.arguments, "name").ok();
                let value = name.as_ref().and_then(|name| std::env::var(name).ok());
                let rendered = match (&name, &value) {
                    (Some(name), Some(value)) => format!("cwd={cwd} {name}={value}"),
                    (Some(name), None) => format!("cwd={cwd} {name}=(unset)"),
                    (None, _) => format!("cwd={cwd}"),
                };
                let mut result = CallToolResult::success(vec![Content::text(rendered)]);
                result.structured_content = Some(json!({
                    "cwd": cwd,
                    "name": name,
                    "value": value,
                }));
                Ok(result)
            }
            other => Err(McpError::invalid_params(
                format!("unknown tool: {other}"),
                None,
            )),
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let startup = format!("cortex-mcp-echo-server started pid={}", std::process::id());
    eprintln!("{startup}");
    if let Ok(path) = std::env::var("CORTEX_MCP_ECHO_LOG") {
        if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
            let _ = writeln!(file, "{startup}");
        }
    }

    let service = EchoServer::new()
        .serve((tokio::io::stdin(), tokio::io::stdout()))
        .await?;
    service.waiting().await?;
    Ok(())
}
