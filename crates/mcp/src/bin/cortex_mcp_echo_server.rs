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
use std::path::PathBuf;
use std::sync::Arc;

use rmcp::ErrorData as McpError;
use rmcp::ServiceExt;
use rmcp::handler::server::ServerHandler;
use rmcp::model::{
    AnnotateAble, Annotations, CallToolRequestParams, CallToolResult, Content,
    GetPromptRequestParams, GetPromptResult, InitializeRequestParams, InitializeResult, JsonObject,
    ListPromptsResult, ListResourcesResult, ListToolsResult, Meta, PaginatedRequestParams, Prompt,
    PromptArgument, PromptMessage, PromptMessageRole, RawAudioContent, RawContent,
    RawEmbeddedResource, RawImageContent, RawResource, RawTextContent, ReadResourceRequestParams,
    ReadResourceResult, ResourceContents, ServerCapabilities, ServerInfo, Tool,
};
use rmcp::service::{RequestContext, RoleServer};
use serde_json::{Value, json};
use url::Url;

#[derive(Clone)]
struct EchoServer {
    label: Option<String>,
    resources_prompts: bool,
    no_args_prompt: bool,
    roots_tool: bool,
    error_tool: bool,
    stall_tool: bool,
    reject_initialize: bool,
    reject_list_tools: bool,
    stall_list_tools: bool,
    resource_content: String,
    collision_tools: bool,
    typed_result: bool,
    extra_tool: Option<String>,
}

impl EchoServer {
    fn new() -> Self {
        Self {
            label: std::env::var("CORTEX_MCP_ECHO_LABEL").ok(),
            resources_prompts: std::env::var("CORTEX_MCP_ECHO_ENABLE_RESOURCES_PROMPTS").is_ok(),
            no_args_prompt: std::env::var("CORTEX_MCP_ECHO_ENABLE_NOARGS_PROMPT").is_ok(),
            roots_tool: std::env::var("CORTEX_MCP_ECHO_ENABLE_ROOTS_TOOL").is_ok(),
            error_tool: std::env::var("CORTEX_MCP_ECHO_ENABLE_ERROR_TOOL").is_ok(),
            stall_tool: std::env::var("CORTEX_MCP_ECHO_ENABLE_STALL_TOOL").is_ok(),
            reject_initialize: std::env::var("CORTEX_MCP_ECHO_REJECT_INITIALIZE").is_ok(),
            reject_list_tools: std::env::var("CORTEX_MCP_ECHO_REJECT_LIST_TOOLS").is_ok(),
            stall_list_tools: std::env::var("CORTEX_MCP_ECHO_STALL_LIST_TOOLS").is_ok(),
            resource_content: std::env::var("CORTEX_MCP_ECHO_RESOURCE_CONTENT")
                .unwrap_or_else(|_| "RESOURCE-CONTENT-default".to_owned()),
            collision_tools: std::env::var("CORTEX_MCP_ECHO_ENABLE_COLLISION_TOOLS").is_ok(),
            typed_result: std::env::var("CORTEX_MCP_ECHO_ENABLE_TYPED_RESULT").is_ok(),
            extra_tool: std::env::var("CORTEX_MCP_ECHO_EXTRA_TOOL")
                .ok()
                .filter(|name| !name.trim().is_empty()),
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
        let mut tools = vec![
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
        ];
        if self.roots_tool {
            tools.push(Tool::new(
                Cow::Borrowed("roots_read"),
                Cow::Borrowed(
                    "Request roots/list from the client and read a UTF-8 file under one of the \
                     advertised roots.",
                ),
                Self::schema(json!({
                    "type": "object",
                    "properties": { "path": { "type": "string" } },
                    "required": ["path"],
                    "additionalProperties": false
                })),
            ));
        }
        if self.error_tool {
            tools.push(Tool::new(
                Cow::Borrowed("boom"),
                Cow::Borrowed("Return an MCP tool error result containing MCP-TOOL-FAILED."),
                Self::schema(json!({
                    "type": "object",
                    "properties": {},
                    "additionalProperties": false
                })),
            ));
        }
        if self.stall_tool {
            tools.push(Tool::new(
                Cow::Borrowed("stall"),
                Cow::Borrowed("Sleep forever to prove the client's tools/call timeout."),
                Self::schema(json!({
                    "type": "object",
                    "properties": {},
                    "additionalProperties": false
                })),
            ));
        }
        for name in self.optional_tool_names() {
            tools.push(Tool::new(
                Cow::Owned(name),
                Cow::Borrowed(
                    "Fixture capability used to prove collision-free model-visible MCP routing.",
                ),
                Self::schema(json!({
                    "type": "object",
                    "properties": {},
                    "additionalProperties": false
                })),
            ));
        }
        tools
    }

    fn optional_tool_names(&self) -> Vec<String> {
        let mut names = self.extra_tool.clone().into_iter().collect::<Vec<_>>();
        if self.typed_result {
            names.push("all_kinds".to_owned());
        }
        if self.collision_tools {
            names.extend([
                "apply_patch".to_owned(),
                "resource_notes://demo".to_owned(),
                "prompt_greet".to_owned(),
            ]);
        }
        names.sort();
        names.dedup();
        names
    }

    fn resources(&self) -> Vec<rmcp::model::Resource> {
        if !self.resources_prompts {
            return Vec::new();
        }
        vec![
            RawResource::new("notes://demo", "demo-notes")
                .with_description("Demo resource exported by the Cortex MCP fixture")
                .with_mime_type("text/plain")
                .no_annotation(),
        ]
    }

    fn prompts(&self) -> Vec<Prompt> {
        if !self.resources_prompts {
            return Vec::new();
        }
        let mut prompts = vec![Prompt::new(
            "greet",
            Some("Greet a caller by name from the MCP fixture."),
            Some(vec![PromptArgument::new("name").with_required(true)]),
        )];
        if self.no_args_prompt {
            prompts.push(Prompt::new(
                "ping",
                Some("Return a no-argument MCP prompt for slash-command fixture coverage."),
                None,
            ));
        }
        prompts
    }

    fn log_call(&self, name: &str, arguments: &Option<JsonObject>) {
        let args = arguments
            .as_ref()
            .map(|map| Value::Object(map.clone()))
            .unwrap_or(Value::Null);
        self.log_line(&format!("tools/call name={name} args={args}"));
    }

    fn log_line(&self, line: &str) {
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
        let mut capabilities = ServerCapabilities::builder().enable_tools().build();
        if self.resources_prompts {
            capabilities = ServerCapabilities::builder()
                .enable_tools()
                .enable_resources()
                .enable_prompts()
                .build();
        }
        info.capabilities = capabilities;
        info
    }

    async fn initialize(
        &self,
        request: InitializeRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<InitializeResult, McpError> {
        if self.reject_initialize {
            // Give the client setup guard an observation interval after this fixture has
            // spawned its descendant. This only makes the intentional failure deterministic.
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            return Err(McpError::internal_error(
                "fixture intentionally rejected initialize",
                None,
            ));
        }
        if context.peer.peer_info().is_none() {
            context.peer.set_peer_info(request);
        }
        Ok(self.get_info())
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, McpError> {
        if self.reject_list_tools {
            return Err(McpError::internal_error(
                "fixture intentionally rejected tools/list",
                None,
            ));
        }
        if self.stall_list_tools {
            std::future::pending::<()>().await;
            unreachable!("pending future never resolves");
        }
        Ok(ListToolsResult {
            tools: self.tools(),
            next_cursor: None,
            meta: None,
        })
    }

    async fn list_resources(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, McpError> {
        Ok(ListResourcesResult {
            resources: self.resources(),
            next_cursor: None,
            meta: None,
        })
    }

    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResult, McpError> {
        self.log_line(&format!("resources/read uri={}", request.uri));
        if request.uri != "notes://demo" {
            return Err(McpError::invalid_params(
                format!("unknown resource: {}", request.uri),
                None,
            ));
        }
        Ok(ReadResourceResult::new(vec![ResourceContents::text(
            self.resource_content.clone(),
            request.uri,
        )]))
    }

    async fn list_prompts(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListPromptsResult, McpError> {
        Ok(ListPromptsResult {
            prompts: self.prompts(),
            next_cursor: None,
            meta: None,
        })
    }

    async fn get_prompt(
        &self,
        request: GetPromptRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<GetPromptResult, McpError> {
        let args = request
            .arguments
            .as_ref()
            .map(|map| Value::Object(map.clone()))
            .unwrap_or(Value::Null);
        self.log_line(&format!("prompts/get name={} args={args}", request.name));
        match request.name.as_str() {
            "greet" => {
                let name = request
                    .arguments
                    .as_ref()
                    .and_then(|map| map.get("name"))
                    .and_then(Value::as_str)
                    .unwrap_or("friend");
                Ok(GetPromptResult::new(vec![PromptMessage::new_text(
                    PromptMessageRole::User,
                    format!("Hello {name} from MCP"),
                )]))
            }
            "ping" if self.no_args_prompt => {
                Ok(GetPromptResult::new(vec![PromptMessage::new_text(
                    PromptMessageRole::User,
                    "Ping from MCP prompt fixture".to_owned(),
                )]))
            }
            _ => Err(McpError::invalid_params(
                format!("unknown prompt: {}", request.name),
                None,
            )),
        }
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
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
            "all_kinds" if self.typed_result => {
                let mut annotations = Annotations::default();
                annotations.priority = Some(0.75);
                let block_meta = || {
                    Meta(JsonObject::from_iter([(
                        "block_meta".to_owned(),
                        json!("fixture"),
                    )]))
                };
                let resource_meta = || {
                    Meta(JsonObject::from_iter([(
                        "resource_meta".to_owned(),
                        json!(true),
                    )]))
                };
                let content = vec![
                    RawContent::Text(RawTextContent {
                        text: "typed-first-text".to_owned(),
                        meta: Some(block_meta()),
                    })
                    .annotate(annotations.clone()),
                    RawContent::Image(RawImageContent {
                        data: "aW1hZ2UtcGF5bG9hZA==".to_owned(),
                        mime_type: "image/png".to_owned(),
                        meta: Some(block_meta()),
                    })
                    .annotate(annotations.clone()),
                    RawContent::Audio(RawAudioContent {
                        data: "YXVkaW8tcGF5bG9hZA==".to_owned(),
                        mime_type: "audio/wav".to_owned(),
                    })
                    .annotate(annotations.clone()),
                    RawContent::Resource(RawEmbeddedResource {
                        meta: Some(block_meta()),
                        resource: ResourceContents::TextResourceContents {
                            uri: "fixture://embedded-text".to_owned(),
                            mime_type: Some("text/plain".to_owned()),
                            text: "typed-embedded-text".to_owned(),
                            meta: Some(resource_meta()),
                        },
                    })
                    .annotate(annotations.clone()),
                    RawContent::Resource(RawEmbeddedResource {
                        meta: Some(block_meta()),
                        resource: ResourceContents::BlobResourceContents {
                            uri: "fixture://embedded-blob".to_owned(),
                            mime_type: Some("application/octet-stream".to_owned()),
                            blob: "YmxvYi1wYXlsb2Fk".to_owned(),
                            meta: Some(resource_meta()),
                        },
                    })
                    .annotate(annotations.clone()),
                    RawContent::ResourceLink(RawResource {
                        uri: "fixture://resource-link".to_owned(),
                        name: "typed-resource-link".to_owned(),
                        title: Some("Typed resource link".to_owned()),
                        description: Some("fixture metadata survives".to_owned()),
                        mime_type: Some("application/octet-stream".to_owned()),
                        size: Some(12),
                        icons: None,
                        meta: Some(block_meta()),
                    })
                    .annotate(annotations),
                ];
                let mut result = CallToolResult::error(content);
                result.structured_content = Some(json!({
                    "nested": [1, { "typed": true }],
                    "source": "all_kinds"
                }));
                result.meta = Some(Meta(JsonObject::from_iter([(
                    "result_meta".to_owned(),
                    json!("fixture"),
                )])));
                Ok(result)
            }
            "boom" if self.error_tool => Ok(CallToolResult::error(vec![Content::text(
                "MCP-TOOL-FAILED",
            )])),
            "stall" if self.stall_tool => {
                std::future::pending::<()>().await;
                unreachable!("pending future never resolves")
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
            "roots_read" if self.roots_tool => {
                let relative = Self::arg_str(&request.arguments, "path")?;
                let capability = context
                    .peer
                    .peer_info()
                    .and_then(|info| info.capabilities.roots.as_ref())
                    .is_some();
                let roots = context
                    .peer
                    .list_roots()
                    .await
                    .map_err(|err| {
                        McpError::internal_error(format!("roots/list request failed: {err}"), None)
                    })?
                    .roots;
                let roots_json = roots
                    .iter()
                    .map(|root| root.uri.clone())
                    .collect::<Vec<_>>();
                let mut chosen_root = None;
                let mut file_text = None;
                for root in &roots {
                    let Some(candidate) = resolve_root_candidate(root, &relative) else {
                        continue;
                    };
                    if !candidate.is_file() {
                        continue;
                    }
                    if let Ok(contents) = std::fs::read_to_string(&candidate) {
                        chosen_root = Some(root.uri.clone());
                        file_text = Some(contents);
                        break;
                    }
                }
                let Some(contents) = file_text else {
                    let mut error = CallToolResult::error(vec![Content::text(format!(
                        "could not resolve `{relative}` under advertised roots"
                    ))]);
                    error.structured_content = Some(json!({
                        "roots_capability": capability,
                        "roots": roots_json,
                    }));
                    return Ok(error);
                };
                let mut result = CallToolResult::success(vec![Content::text(contents.clone())]);
                result.structured_content = Some(json!({
                    "roots_capability": capability,
                    "roots": roots_json,
                    "root": chosen_root,
                    "path": relative,
                    "content": contents,
                }));
                Ok(result)
            }
            other if self.optional_tool_names().iter().any(|name| name == other) => {
                let label = self.label.clone().unwrap_or_else(|| "default".to_owned());
                let text = format!("{label}: collision tool {other}");
                let mut result = CallToolResult::success(vec![Content::text(text)]);
                result.structured_content = Some(json!({ "label": label, "tool": other }));
                Ok(result)
            }
            other => Err(McpError::invalid_params(
                format!("unknown tool: {other}"),
                None,
            )),
        }
    }
}

fn resolve_root_candidate(root: &rmcp::model::Root, relative: &str) -> Option<PathBuf> {
    let url = Url::parse(&root.uri).ok()?;
    if url.scheme() != "file" {
        return None;
    }
    let root_path = url.to_file_path().ok()?;
    let mut candidate = root_path.join(relative);
    if let Ok(canonical) = std::fs::canonicalize(&candidate) {
        candidate = canonical;
    }
    if !candidate.starts_with(&root_path) {
        return None;
    }
    Some(candidate)
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::var("CORTEX_MCP_ECHO_EXIT_IMMEDIATELY").is_ok() {
        return Ok(());
    }

    let startup_cwd = std::env::current_dir()
        .map(|path| path.display().to_string())
        .unwrap_or_default();
    let configured_flag =
        std::env::var("CORTEX_MCP_ECHO_CONFIGURED_FLAG").unwrap_or_else(|_| "(unset)".to_owned());
    let startup_probe = std::env::var("CORTEX_MCP_ECHO_STARTUP_ENV_NAME")
        .ok()
        .map(|name| {
            let value = std::env::var(&name).unwrap_or_else(|_| "(unset)".to_owned());
            format!(" {name}={value}")
        })
        .unwrap_or_default();
    let startup = format!(
        "cortex-mcp-echo-server started pid={} cwd={startup_cwd} \
         CORTEX_MCP_ECHO_CONFIGURED_FLAG={configured_flag}{startup_probe}",
        std::process::id()
    );
    eprintln!("{startup}");
    if let Ok(path) = std::env::var("CORTEX_MCP_ECHO_LOG") {
        if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
            let _ = writeln!(file, "{startup}");
        }
    }

    // Optionally fork a long-lived grandchild so a black-box validator can prove the whole process
    // tree (not just the direct child) is terminated by PID on shutdown. The grandchild's PID is
    // written to the requested file so the test can assert it is reaped.
    if std::env::var("CORTEX_MCP_ECHO_SPAWN_CHILD").is_ok() {
        if let Ok(child) = std::process::Command::new("sleep").arg("600").spawn() {
            if let Ok(path) = std::env::var("CORTEX_MCP_ECHO_CHILD_PID_FILE") {
                let _ = std::fs::write(path, child.id().to_string());
            }
            std::mem::forget(child);
        }
    }

    if std::env::var("CORTEX_MCP_ECHO_STALL_BEFORE_SERVE").is_ok() {
        std::future::pending::<()>().await;
    }

    let service = EchoServer::new()
        .serve((tokio::io::stdin(), tokio::io::stdout()))
        .await?;
    service.waiting().await?;
    Ok(())
}
