//! Agent Client Protocol (ACP) stdio surface.
//!
//! `cortex acp` is a thin JSON-RPC shell over the same durable runner used by the other surfaces:
//! it creates sessions in the shared SQLite store, runs real OpenRouter turns through
//! [`engine::SessionRunner`], streams assistant text as ACP `session/update` notifications, and
//! cancels in-flight turns by dropping the runner future at the stdio boundary.

use std::collections::BTreeMap;
use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex as StdMutex};

use agent_client_protocol::schema::ProtocolVersion;
use agent_client_protocol::schema::v1::{
    AgentCapabilities, CancelNotification, ContentBlock, ContentChunk, EmbeddedResourceResource,
    Implementation, InitializeRequest, InitializeResponse, MessageId, NewSessionRequest,
    NewSessionResponse, PromptCapabilities, PromptRequest, PromptResponse, SessionCapabilities,
    SessionId, SessionNotification, SessionUpdate, StopReason, TextContent,
};
use db::{Database, DbPathInputs};
use engine::{
    CompactionPolicy, EnvironmentBlock, HistoryMessage, ModelFamily, PartLifecycleTracker,
    RunOutcome, RunnerEvent, SessionRunner, SystemContext, ToolDispatchResult, ToolDispatcher,
    Turn, TurnQueue, build_assistant_message_parts, project_history,
};
use errors::Exit;
use events::{Event, EventBus};
use futures_util::future::BoxFuture;
use llm::{Provider, ToolCall, ToolDef};
use permissions::PermissionEngine;
use protocol::{MessagePart as RichMessagePart, TodoItem};
use providers::ResolvedModel;
use redaction::guard_outbound;
use serde_json::{Value, json};
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncRead, AsyncReadExt, BufReader};
use tokio::sync::watch;
use tools::{ToolContext, ToolRegistry};

const OUTBOUND_OPERATION: &str = "ACP LLM turn";

/// Run the ACP stdio server in a fresh Tokio runtime.
pub fn run_stdio(bus: Arc<EventBus>) -> Exit {
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(err) => {
            eprintln!("cortex acp: failed to initialize async runtime: {err}");
            return Exit::Failure;
        }
    };

    runtime.block_on(async move {
        let service = match AcpService::from_process(bus) {
            Ok(service) => service,
            Err(message) => {
                eprintln!("cortex acp: {message}");
                return Exit::Failure;
            }
        };
        let writer = JsonRpcWriter::new(std::io::stdout());
        match service.run_loop(tokio::io::stdin(), writer).await {
            Ok(()) => Exit::Success,
            Err(err) => {
                eprintln!("cortex acp: {err}");
                Exit::Failure
            }
        }
    })
}

#[derive(Clone)]
pub struct AcpService {
    state: Arc<AcpState>,
}

struct AcpState {
    db_path: PathBuf,
    provider: ProviderSource,
    event_bus: Arc<EventBus>,
    active_turns: TurnQueue,
    initialized: StdMutex<bool>,
}

#[derive(Clone)]
enum ProviderSource {
    OpenRouter,
    #[cfg(test)]
    Fixed {
        model_id: String,
        provider: Arc<dyn Provider>,
    },
}

impl AcpService {
    /// Build an ACP service from the process environment and shared database location.
    pub fn from_process(event_bus: Arc<EventBus>) -> Result<Self, String> {
        let Some(db_path) = DbPathInputs::from_process().resolve() else {
            return Err(
                "cannot determine a database location; set HOME, XDG_DATA_HOME, CORTEX_DATA_DIR, or CORTEX_DB_PATH"
                    .to_owned(),
            );
        };
        Database::open(&db_path).map_err(|err| format!("cannot open session database: {err}"))?;
        Ok(Self::new(db_path, ProviderSource::OpenRouter, event_bus))
    }

    #[cfg(test)]
    fn with_provider(
        db_path: PathBuf,
        model_id: impl Into<String>,
        provider: Arc<dyn Provider>,
    ) -> Self {
        Self::new(
            db_path,
            ProviderSource::Fixed {
                model_id: model_id.into(),
                provider,
            },
            Arc::new(EventBus::new()),
        )
    }

    fn new(db_path: PathBuf, provider: ProviderSource, event_bus: Arc<EventBus>) -> Self {
        Self {
            state: Arc::new(AcpState {
                db_path,
                provider,
                event_bus,
                active_turns: TurnQueue::default(),
                initialized: StdMutex::new(false),
            }),
        }
    }

    /// Drive JSON-RPC messages from an async reader and write responses/notifications to `writer`.
    pub async fn run_loop<R, W>(&self, reader: R, writer: JsonRpcWriter<W>) -> io::Result<()>
    where
        R: AsyncRead + Unpin,
        W: Write + Send + 'static,
    {
        let mut reader = BufReader::new(reader);
        loop {
            let frame = match read_frame(&mut reader).await {
                Ok(Some(frame)) => frame,
                Ok(None) => return Ok(()),
                Err(err) if err.kind() == io::ErrorKind::InvalidData => {
                    writer.send_error(Value::Null, -32700, format!("parse error: {err}"))?;
                    continue;
                }
                Err(err) => return Err(err),
            };
            let value = match serde_json::from_str::<Value>(&frame) {
                Ok(value) => value,
                Err(err) => {
                    writer.send_error(Value::Null, -32700, format!("parse error: {err}"))?;
                    continue;
                }
            };
            self.handle_value(value, writer.clone())?;
        }
    }

    fn handle_value<W>(&self, value: Value, writer: JsonRpcWriter<W>) -> io::Result<()>
    where
        W: Write + Send + 'static,
    {
        let Some(object) = value.as_object() else {
            return writer.send_error(Value::Null, -32600, "invalid JSON-RPC request");
        };
        if !object.contains_key("method")
            && (object.contains_key("result") || object.contains_key("error"))
        {
            return Ok(());
        }
        let id = match object.get("id") {
            Some(id) if is_valid_json_rpc_id(id) => Some(id.clone()),
            Some(_) => {
                return writer.send_error(
                    Value::Null,
                    -32600,
                    "invalid JSON-RPC request: id must be a string, number, or null",
                );
            }
            None => None,
        };
        let response_id = id.clone().unwrap_or(Value::Null);
        if object.get("jsonrpc") != Some(&json!("2.0")) {
            return writer.send_error(
                response_id,
                -32600,
                "invalid JSON-RPC request: jsonrpc must be \"2.0\"",
            );
        }
        let method = match object.get("method") {
            Some(Value::String(method)) => method.as_str(),
            Some(_) => {
                return writer.send_error(
                    response_id,
                    -32600,
                    "invalid JSON-RPC request: method must be a string",
                );
            }
            None => {
                return writer.send_error(
                    response_id,
                    -32600,
                    "invalid JSON-RPC request: missing method",
                );
            }
        };
        let params = object.get("params").cloned().unwrap_or(Value::Null);
        if let Some(id) = id {
            self.handle_request(id, method, params, writer)
        } else {
            self.handle_notification(method, params, writer)
        }
    }

    fn handle_request<W>(
        &self,
        id: Value,
        method: &str,
        params: Value,
        writer: JsonRpcWriter<W>,
    ) -> io::Result<()>
    where
        W: Write + Send + 'static,
    {
        match method {
            "initialize" => self.initialize(id, params, writer),
            "session/new" => {
                if !self.is_initialized() {
                    return writer.send_error(id, -32002, "ACP connection is not initialized");
                }
                self.new_session(id, params, writer)
            }
            "session/prompt" => {
                if !self.is_initialized() {
                    return writer.send_error(id, -32002, "ACP connection is not initialized");
                }
                self.start_prompt(id, params, writer)
            }
            "session/cancel" => {
                if !self.is_initialized() {
                    return writer.send_error(id, -32002, "ACP connection is not initialized");
                }
                let session_id = match parse_cancel_session_id(params) {
                    Ok(session_id) => session_id,
                    Err(message) => return writer.send_error(id, -32602, message),
                };
                let active = match self.state.active_turns.abort(&session_id) {
                    Ok(active) => active,
                    Err(message) => return writer.send_error(id, -32000, message),
                };
                writer.send_result(id, json!({ "cancelled": active, "active": active }))
            }
            _ => writer.send_error(id, -32601, format!("method not found: {method}")),
        }
    }

    fn handle_notification<W>(
        &self,
        method: &str,
        params: Value,
        _writer: JsonRpcWriter<W>,
    ) -> io::Result<()>
    where
        W: Write + Send + 'static,
    {
        if method == "session/cancel"
            && self.is_initialized()
            && let Ok(session_id) = parse_cancel_session_id(params)
        {
            let _ = self.state.active_turns.abort(&session_id);
        }
        Ok(())
    }

    fn initialize<W>(&self, id: Value, params: Value, writer: JsonRpcWriter<W>) -> io::Result<()>
    where
        W: Write,
    {
        let request = match serde_json::from_value::<InitializeRequest>(params) {
            Ok(request) => request,
            Err(err) => {
                return writer.send_error(id, -32602, format!("invalid initialize params: {err}"));
            }
        };
        let requested = request.protocol_version;
        let protocol_version =
            if requested.as_u16() == 0 || requested.as_u16() > ProtocolVersion::LATEST.as_u16() {
                ProtocolVersion::LATEST
            } else {
                requested
            };
        let capabilities = AgentCapabilities::new()
            .prompt_capabilities(PromptCapabilities::new().embedded_context(true))
            .session_capabilities(SessionCapabilities::new());
        let response = InitializeResponse::new(protocol_version)
            .agent_capabilities(capabilities)
            .agent_info(Some(
                Implementation::new("cortex", env!("CARGO_PKG_VERSION"))
                    .title(Some("Cortex Code".to_owned())),
            ));
        *self.state.initialized.lock().expect("initialized mutex") = true;
        writer.send_result(
            id,
            serde_json::to_value(response).expect("serialize initialize"),
        )
    }

    fn new_session<W>(&self, id: Value, params: Value, writer: JsonRpcWriter<W>) -> io::Result<()>
    where
        W: Write,
    {
        let _request = match parse_new_session_params(params) {
            Ok(request) => request,
            Err(message) => return writer.send_error(id, -32602, message),
        };
        let db = match Database::open(&self.state.db_path) {
            Ok(db) => db,
            Err(err) => return writer.send_error(id, -32000, err.to_string()),
        };
        let session = match db.sessions().create(None) {
            Ok(session) => session,
            Err(err) => return writer.send_error(id, -32000, err.to_string()),
        };
        let response = NewSessionResponse::new(session.id);
        writer.send_result(
            id,
            serde_json::to_value(response).expect("serialize session/new"),
        )
    }

    fn start_prompt<W>(&self, id: Value, params: Value, writer: JsonRpcWriter<W>) -> io::Result<()>
    where
        W: Write + Send + 'static,
    {
        let prompt = match parse_prompt_params(params) {
            Ok(prompt) => prompt,
            Err(message) => return writer.send_error(id, -32602, message),
        };
        let service = self.clone();
        tokio::spawn(async move {
            service.run_prompt(id, prompt, writer).await;
        });
        Ok(())
    }

    async fn run_prompt<W>(&self, id: Value, prompt: ParsedPrompt, writer: JsonRpcWriter<W>)
    where
        W: Write + Send + 'static,
    {
        if let Err(diagnostic) = guard_outbound(OUTBOUND_OPERATION) {
            let _ = writer.send_error(id, -32000, diagnostic.to_string());
            return;
        }

        let db = match Database::open(&self.state.db_path) {
            Ok(db) => db,
            Err(err) => {
                let _ = writer.send_error(id, -32000, err.to_string());
                return;
            }
        };
        let session = match db.sessions().require(&prompt.session_id) {
            Ok(session) => session,
            Err(err) => {
                let _ = writer.send_error(id, -32000, err.to_string());
                return;
            }
        };

        let active_turn = match self
            .state
            .active_turns
            .acquire(prompt.session_id.clone())
            .await
        {
            Ok(lease) => lease,
            Err(message) => {
                let _ = writer.send_error(id, -32000, message);
                return;
            }
        };
        let queued = active_turn.queued();

        let resolved = match self.resolve_turn_provider(session.model.as_deref()) {
            Ok(resolved) => resolved,
            Err(message) => {
                let _ = writer.send_error(id, -32000, message);
                return;
            }
        };
        let history = match load_history(&db, &prompt.session_id) {
            Ok(history) => history,
            Err(message) => {
                let _ = writer.send_error(id, -32000, message);
                return;
            }
        };
        let user_message = match db.messages().create_with_parts(
            &prompt.session_id,
            "user",
            &prompt.text,
            &[RichMessagePart::text(prompt.text.clone())],
        ) {
            Ok(message) => message,
            Err(err) => {
                let _ = writer.send_error(id, -32000, err.to_string());
                return;
            }
        };
        self.state
            .event_bus
            .publish(&Event::turn_started(&prompt.session_id, &user_message.id));

        let system =
            SystemContext::for_model(&resolved.model_id, EnvironmentBlock::from_process()).render();
        let turn = Turn::new(&resolved.model_id, system, prompt.text.clone())
            .with_history(history)
            .with_compaction(compaction_policy(&resolved.model));
        let permissions = match permission_engine_for_turn(
            self.state.event_bus.clone(),
            &prompt.session_id,
            &user_message.id,
        ) {
            Ok(permissions) => permissions,
            Err(message) => {
                let _ = writer.send_error(id, -32000, message);
                return;
            }
        };
        let dispatcher = AcpToolDispatcher::for_model(
            std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            &resolved.model_id,
            prompt.session_id.clone(),
            self.state.db_path.clone(),
            permissions,
        );
        let result = run_cancellable_turn(
            resolved.provider.as_ref(),
            turn,
            &dispatcher,
            active_turn.abort_receiver(),
            TurnEventSink {
                bus: self.state.event_bus.clone(),
                session_id: prompt.session_id.clone(),
                message_id: user_message.id.clone(),
                writer: writer.clone(),
            },
        )
        .await;
        dispatcher.shutdown().await;

        let Some(result) = result else {
            self.state.event_bus.publish(&Event::turn_done(
                &prompt.session_id,
                &user_message.id,
                "cancelled",
                0,
            ));
            let _ = writer.send_result(id, prompt_response_json(StopReason::Cancelled, queued));
            return;
        };
        let TurnRunResult {
            outcome,
            runner_events,
        } = result;

        if let Some(error) = &outcome.error {
            let _ = writer.send_error(id, -32000, error.to_string());
            return;
        }

        persist_usage(&db, &prompt.session_id, &outcome);
        let mut lifecycle = PartLifecycleTracker::new(&user_message.id, &resolved.model_id);
        for event in &runner_events {
            let _ = lifecycle.observe(event);
        }
        let part_timestamps = lifecycle.part_timestamps();
        let assistant_parts = build_assistant_message_parts(
            &resolved.model_id,
            "general",
            &user_message.id,
            &runner_events,
            &outcome,
            &part_timestamps,
        );
        let assistant = match db.messages().create_with_parts(
            &prompt.session_id,
            "assistant",
            outcome.answer(),
            &assistant_parts,
        ) {
            Ok(message) => message,
            Err(err) => {
                let _ = writer.send_error(id, -32000, err.to_string());
                return;
            }
        };
        self.state.event_bus.publish(&Event::message_parts(
            prompt.session_id.clone(),
            assistant.id.clone(),
            assistant_parts.clone(),
        ));
        let parts_note = SessionNotification::new(
            SessionId::new(prompt.session_id.clone()),
            SessionUpdate::AgentThoughtChunk(
                ContentChunk::new(ContentBlock::Text(TextContent::new(
                    serde_json::json!({
                        "type": "assistant_parts",
                        "message_id": assistant.id,
                        "parts": assistant_parts,
                    })
                    .to_string(),
                )))
                .message_id(Some(MessageId::new(format!("{}-parts", user_message.id)))),
            ),
        );
        let _ = writer.send_notification(
            "session/update",
            serde_json::to_value(parts_note).expect("serialize parts update"),
        );

        let _ = writer.send_result(
            id,
            prompt_response_json(stop_reason_for_outcome(&outcome), queued),
        );
    }

    fn resolve_turn_provider(
        &self,
        session_model: Option<&str>,
    ) -> Result<ResolvedProvider, String> {
        match &self.state.provider {
            #[cfg(test)]
            ProviderSource::Fixed { model_id, provider } => {
                let model = providers::resolve_model(Some(model_id));
                Ok(ResolvedProvider {
                    model_id: model.model_id.clone(),
                    model,
                    provider: provider.clone(),
                })
            }
            ProviderSource::OpenRouter => {
                let config_inputs = settings::ConfigInputs::from_process(BTreeMap::new())
                    .map_err(|err| format!("cannot load configuration context: {err}"))?;
                let effective = settings::load(config_inputs)
                    .map_err(|err| format!("cannot load configuration: {err}"))?;
                let model_id = session_model
                    .map(str::trim)
                    .filter(|model| !model.is_empty())
                    .unwrap_or_else(|| effective.model());
                validate_model_id(model_id)?;
                let model = providers::resolve_model(Some(model_id));
                let inputs = providers::CredentialInputs::openrouter_from_process(
                    openrouter_api_key(&effective),
                );
                let provider = providers::resolve_openrouter_with(
                    &inputs,
                    openrouter_base_url_override(&effective).as_deref(),
                )
                .map_err(|diagnostic| diagnostic.to_string())?;
                Ok(ResolvedProvider {
                    model_id: model.model_id.clone(),
                    model,
                    provider: Arc::new(provider),
                })
            }
        }
    }

    fn is_initialized(&self) -> bool {
        *self.state.initialized.lock().expect("initialized mutex")
    }
}

struct ResolvedProvider {
    model_id: String,
    model: ResolvedModel,
    provider: Arc<dyn Provider>,
}

pub struct JsonRpcWriter<W> {
    inner: Arc<StdMutex<W>>,
}

impl<W> Clone for JsonRpcWriter<W> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

impl<W> JsonRpcWriter<W> {
    pub fn new(writer: W) -> Self {
        Self {
            inner: Arc::new(StdMutex::new(writer)),
        }
    }
}

impl<W> JsonRpcWriter<W>
where
    W: Write,
{
    fn send_result(&self, id: Value, result: Value) -> io::Result<()> {
        self.send_value(json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": result,
        }))
    }

    fn send_error(&self, id: Value, code: i64, message: impl Into<String>) -> io::Result<()> {
        self.send_value(json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": {
                "code": code,
                "message": message.into(),
            },
        }))
    }

    fn send_notification(&self, method: &str, params: Value) -> io::Result<()> {
        self.send_value(json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
        }))
    }

    fn send_value(&self, value: Value) -> io::Result<()> {
        let mut writer = self.inner.lock().expect("json-rpc writer mutex");
        serde_json::to_writer(&mut *writer, &value)?;
        writer.write_all(b"\n")?;
        writer.flush()
    }
}

async fn read_frame<R>(reader: &mut R) -> io::Result<Option<String>>
where
    R: AsyncBufRead + Unpin,
{
    loop {
        let mut line = String::new();
        let read = reader.read_line(&mut line).await?;
        if read == 0 {
            return Ok(None);
        }
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed.is_empty() {
            continue;
        }
        if let Some(length) = trimmed.strip_prefix("Content-Length:") {
            let length = length.trim().parse::<usize>().map_err(|err| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("invalid Content-Length header: {err}"),
                )
            })?;
            loop {
                let mut header = String::new();
                let read = reader.read_line(&mut header).await?;
                if read == 0 || header.trim_end_matches(['\r', '\n']).is_empty() {
                    break;
                }
            }
            let mut body = vec![0_u8; length];
            reader.read_exact(&mut body).await?;
            return String::from_utf8(body)
                .map(Some)
                .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err));
        }
        return Ok(Some(trimmed.to_owned()));
    }
}

struct ParsedPrompt {
    session_id: String,
    text: String,
}

fn is_valid_json_rpc_id(value: &Value) -> bool {
    value.is_null() || value.is_string() || value.is_number()
}

fn parse_new_session_params(params: Value) -> Result<NewSessionRequest, String> {
    serde_json::from_value(params).map_err(|err| format!("invalid session/new params: {err}"))
}

fn parse_prompt_params(params: Value) -> Result<ParsedPrompt, String> {
    if let Ok(request) = serde_json::from_value::<PromptRequest>(params.clone()) {
        let text = content_blocks_to_text(&request.prompt);
        if !text.trim().is_empty() {
            return Ok(ParsedPrompt {
                session_id: request.session_id.0.to_string(),
                text,
            });
        }
    }

    let Some(object) = params.as_object() else {
        return Err("session/prompt params must be an object".to_owned());
    };
    let session_id = object
        .get("sessionId")
        .or_else(|| object.get("session_id"))
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| "session/prompt params must include sessionId".to_owned())?;
    let text = object
        .get("text")
        .or_else(|| object.get("content"))
        .or_else(|| object.get("message"))
        .and_then(Value::as_str)
        .map(str::to_owned)
        .or_else(|| object.get("prompt").and_then(prompt_value_to_text))
        .ok_or_else(|| "session/prompt params must include text or prompt content".to_owned())?;
    if text.trim().is_empty() {
        return Err("session/prompt text cannot be empty".to_owned());
    }
    Ok(ParsedPrompt { session_id, text })
}

fn prompt_value_to_text(value: &Value) -> Option<String> {
    if let Some(text) = value.as_str() {
        return Some(text.to_owned());
    }
    let blocks = value.as_array()?;
    let text = blocks
        .iter()
        .filter_map(|block| {
            if let Ok(content) = serde_json::from_value::<ContentBlock>(block.clone()) {
                return Some(content_block_to_text(&content));
            }
            block.get("text").and_then(Value::as_str).map(str::to_owned)
        })
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    (!text.trim().is_empty()).then_some(text)
}

fn content_blocks_to_text(blocks: &[ContentBlock]) -> String {
    blocks
        .iter()
        .map(content_block_to_text)
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

fn content_block_to_text(block: &ContentBlock) -> String {
    match block {
        ContentBlock::Text(text) => text.text.clone(),
        ContentBlock::Resource(resource) => match &resource.resource {
            EmbeddedResourceResource::TextResourceContents(text) => text.text.clone(),
            _ => String::new(),
        },
        ContentBlock::ResourceLink(link) => link.uri.clone(),
        _ => String::new(),
    }
}

fn parse_cancel_session_id(params: Value) -> Result<String, String> {
    if let Ok(cancel) = serde_json::from_value::<CancelNotification>(params.clone()) {
        return Ok(cancel.session_id.0.to_string());
    }
    params
        .get("sessionId")
        .or_else(|| params.get("session_id"))
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| "session/cancel params must include sessionId".to_owned())
}

struct TurnEventSink<W> {
    bus: Arc<EventBus>,
    session_id: String,
    message_id: String,
    writer: JsonRpcWriter<W>,
}

async fn run_cancellable_turn<W>(
    provider: &dyn Provider,
    turn: Turn,
    dispatcher: &dyn ToolDispatcher,
    mut abort_rx: watch::Receiver<bool>,
    events: TurnEventSink<W>,
) -> Option<TurnRunResult>
where
    W: Write + Send + 'static,
{
    let runner = SessionRunner::default();
    let notify = events.writer.clone();
    let bus = events.bus;
    let session_id = events.session_id;
    let message_id = events.message_id;
    let captured_events: Arc<StdMutex<Vec<RunnerEvent>>> = Arc::new(StdMutex::new(Vec::new()));
    let captured_for_sink = captured_events.clone();
    tokio::select! {
        outcome = runner.run_with_tools(provider, turn, dispatcher, move |event| {
            publish_runner_event(&bus, &session_id, &message_id, &event);
            write_acp_update(&notify, &session_id, &message_id, &event);
            if let Ok(mut captured) = captured_for_sink.lock() {
                captured.push(event.clone());
            }
        }) => Some(outcome),
        () = wait_for_abort(&mut abort_rx) => None,
    }
    .map(|outcome| TurnRunResult {
        outcome,
        runner_events: captured_events
            .lock()
            .map(|captured| captured.clone())
            .unwrap_or_default(),
    })
}

struct TurnRunResult {
    outcome: RunOutcome,
    runner_events: Vec<RunnerEvent>,
}

fn write_acp_update<W>(
    writer: &JsonRpcWriter<W>,
    session_id: &str,
    message_id: &str,
    event: &RunnerEvent,
) where
    W: Write,
{
    let update = match event {
        RunnerEvent::TextDelta { text } if !text.is_empty() => {
            Some(SessionUpdate::AgentMessageChunk(
                ContentChunk::new(ContentBlock::Text(TextContent::new(text.clone())))
                    .message_id(Some(MessageId::new(message_id.to_owned()))),
            ))
        }
        RunnerEvent::Reasoning { text } if !text.is_empty() => {
            Some(SessionUpdate::AgentThoughtChunk(
                ContentChunk::new(ContentBlock::Text(TextContent::new(text.clone())))
                    .message_id(Some(MessageId::new(format!("{message_id}-thought")))),
            ))
        }
        RunnerEvent::ToolCall {
            id,
            name,
            arguments,
        } => Some(SessionUpdate::AgentThoughtChunk(
            ContentChunk::new(ContentBlock::Text(TextContent::new(
                serde_json::json!({
                    "type": "tool",
                    "state": "pending",
                    "call_id": id,
                    "name": name,
                    "arguments": arguments,
                })
                .to_string(),
            )))
            .message_id(Some(MessageId::new(format!("{message_id}-tool-{id}")))),
        )),
        RunnerEvent::ToolResult {
            id,
            name,
            parts,
            is_error,
        } => Some(SessionUpdate::AgentThoughtChunk(
            ContentChunk::new(ContentBlock::Text(TextContent::new(
                serde_json::json!({
                    "type": "tool",
                    "state": if *is_error { "error" } else { "completed" },
                    "call_id": id,
                    "name": name,
                    "parts": parts,
                })
                .to_string(),
            )))
            .message_id(Some(MessageId::new(format!(
                "{message_id}-tool-result-{id}"
            )))),
        )),
        RunnerEvent::Retry {
            attempt,
            max_attempts,
            delay_ms,
            error_kind,
            classification,
            message,
        } => Some(SessionUpdate::AgentThoughtChunk(
            ContentChunk::new(ContentBlock::Text(TextContent::new(
                serde_json::json!({
                    "type": "retry",
                    "attempt": attempt,
                    "max_attempts": max_attempts,
                    "delay_ms": delay_ms,
                    "error_kind": error_kind,
                    "classification": classification,
                    "message": message,
                })
                .to_string(),
            )))
            .message_id(Some(MessageId::new(format!(
                "{message_id}-retry-{attempt}"
            )))),
        )),
        RunnerEvent::Usage(usage) => Some(SessionUpdate::AgentThoughtChunk(
            ContentChunk::new(ContentBlock::Text(TextContent::new(
                serde_json::json!({
                    "type": "usage",
                    "input_tokens": usage.input_tokens,
                    "output_tokens": usage.output_tokens,
                    "total_tokens": usage.total_tokens,
                    "cost": usage.cost,
                })
                .to_string(),
            )))
            .message_id(Some(MessageId::new(format!("{message_id}-usage")))),
        )),
        _ => None,
    };
    if let Some(update) = update {
        let notification = SessionNotification::new(SessionId::new(session_id.to_owned()), update);
        let _ = writer.send_notification(
            "session/update",
            serde_json::to_value(notification).expect("serialize session/update"),
        );
    }
}

fn publish_runner_event(bus: &EventBus, session_id: &str, message_id: &str, event: &RunnerEvent) {
    match event {
        RunnerEvent::TextDelta { text } => bus.publish(&Event::assistant_text_delta(
            session_id.to_owned(),
            message_id.to_owned(),
            text.clone(),
        )),
        RunnerEvent::Reasoning { text } => bus.publish(&Event::assistant_reasoning_delta(
            session_id.to_owned(),
            message_id.to_owned(),
            text.clone(),
        )),
        RunnerEvent::ToolCall {
            id,
            name,
            arguments,
        } => bus.publish(&Event::tool_call_for_turn(
            session_id.to_owned(),
            message_id.to_owned(),
            id.clone(),
            name.clone(),
            arguments.clone(),
        )),
        RunnerEvent::ToolResult {
            id, name, is_error, ..
        } => bus.publish(&Event::tool_result_for_turn(
            session_id.to_owned(),
            message_id.to_owned(),
            id.clone(),
            name.clone(),
            *is_error,
        )),
        RunnerEvent::Usage(usage) => bus.publish(&Event::turn_usage(
            session_id.to_owned(),
            message_id.to_owned(),
            usage.input_tokens,
            usage.output_tokens,
            usage.total_tokens,
            usage.cost,
        )),
        RunnerEvent::Error { message, kind } => bus.publish(&Event::turn_error(
            session_id.to_owned(),
            message_id.to_owned(),
            kind.clone(),
            message.clone(),
        )),
        RunnerEvent::Done { reason, steps } => bus.publish(&Event::turn_done(
            session_id.to_owned(),
            message_id.to_owned(),
            finish_reason_name(reason),
            *steps,
        )),
        RunnerEvent::ToolSetChanged { .. } => {}
        RunnerEvent::Retry { .. } => {}
    }
}

fn finish_reason_name(reason: &llm::FinishReason) -> String {
    serde_json::to_value(reason)
        .ok()
        .and_then(|value| value.as_str().map(ToOwned::to_owned))
        .unwrap_or_else(|| format!("{reason:?}"))
}

async fn wait_for_abort(abort_rx: &mut watch::Receiver<bool>) {
    loop {
        if *abort_rx.borrow() {
            break;
        }
        if abort_rx.changed().await.is_err() {
            break;
        }
    }
}

fn load_history(db: &Database, session_id: &str) -> Result<Vec<llm::Message>, String> {
    db.messages()
        .list_for_session(session_id)
        .map_err(|err| err.to_string())
        .map(|messages| {
            messages
                .into_iter()
                .map(|message| {
                    let content = if message.content.trim().is_empty() {
                        message
                            .parts
                            .iter()
                            .filter_map(|part| match &part.body {
                                protocol::MessagePartBody::Text { text }
                                | protocol::MessagePartBody::Reasoning { text } => {
                                    Some(text.as_str())
                                }
                                _ => None,
                            })
                            .collect::<Vec<_>>()
                            .join("\n")
                    } else {
                        message.content.clone()
                    };
                    HistoryMessage::new(message.role, content)
                })
                .collect::<Vec<_>>()
        })
        .map(|history| project_history(&history))
}

fn persist_usage(db: &Database, session_id: &str, outcome: &RunOutcome) {
    let Some(usage) = outcome.usage.as_ref() else {
        return;
    };
    if let Err(err) = db.usage().record(
        session_id,
        u64::from(usage.input_tokens),
        u64::from(usage.output_tokens),
        u64::from(usage.total_tokens),
        usage.cost,
    ) {
        eprintln!("cortex acp: failed to persist usage for session {session_id}: {err}");
    }
}

fn compaction_policy(model: &ResolvedModel) -> CompactionPolicy {
    let (context, output) = match &model.info {
        Some(info) => (
            usize::try_from(info.limit.context).unwrap_or(usize::MAX),
            usize::try_from(info.limit.output).unwrap_or(0),
        ),
        None => (engine::DEFAULT_CONTEXT_LIMIT_TOKENS, 0),
    };
    CompactionPolicy::for_context_window(context, output)
}

fn validate_model_id(model_id: &str) -> Result<(), String> {
    providers::validate_model_id(model_id)
}

fn stop_reason_for_outcome(outcome: &RunOutcome) -> StopReason {
    match outcome.finish_reason {
        llm::FinishReason::Length => StopReason::MaxTokens,
        llm::FinishReason::ContentFilter | llm::FinishReason::Error => StopReason::Refusal,
        llm::FinishReason::Stop | llm::FinishReason::ToolCalls | llm::FinishReason::Other(_) => {
            StopReason::EndTurn
        }
    }
}

fn prompt_response_json(stop_reason: StopReason, queued: bool) -> Value {
    let mut value =
        serde_json::to_value(PromptResponse::new(stop_reason)).expect("serialize prompt response");
    if let Some(object) = value.as_object_mut() {
        object.insert("queued".to_owned(), Value::Bool(queued));
    }
    value
}

fn permission_engine_for_turn(
    bus: Arc<EventBus>,
    session_id: &str,
    message_id: &str,
) -> Result<Option<Arc<PermissionEngine>>, String> {
    let inputs = settings::ConfigInputs::from_process(BTreeMap::new())
        .map_err(|err| format!("cannot load permission configuration context: {err}"))?;
    let mut ruleset = permissions::Ruleset::new(permissions::permissive_default_rules());
    let loaded = permissions::load_layered(&inputs)
        .map_err(|err| format!("invalid permission configuration: {err}"))?;
    for rule in loaded.rules() {
        ruleset.push(rule.clone());
    }
    let trust =
        policy::resolve_trust(&inputs).map_err(|err| format!("cannot load trust policy: {err}"))?;
    for rule in policy::trust_ruleset(trust).rules() {
        ruleset.push(rule.clone());
    }
    Ok(Some(Arc::new(
        PermissionEngine::new(ruleset)
            .with_bus(bus)
            .with_event_context(session_id.to_owned(), message_id.to_owned()),
    )))
}

struct AcpToolDispatcher {
    registry: ToolRegistry,
    context: ToolContext,
}

impl AcpToolDispatcher {
    fn for_model(
        cwd: PathBuf,
        model_id: &str,
        session_id: String,
        db_path: PathBuf,
        permissions: Option<Arc<PermissionEngine>>,
    ) -> Self {
        let registry = match ModelFamily::from_model_id(model_id) {
            ModelFamily::Gpt | ModelFamily::Codex => ToolRegistry::gpt(),
            ModelFamily::Anthropic | ModelFamily::Gemini | ModelFamily::Default => {
                ToolRegistry::builtin()
            }
        };
        let mut context = ToolContext::new(cwd)
            .with_session(session_id)
            .with_todo_store(Arc::new(AcpTodoStore { db_path }));
        if let Some(permissions) = permissions {
            context = context.with_permissions(permissions);
        }
        Self { registry, context }
    }

    async fn shutdown(&self) {
        self.context.background_jobs().shutdown().await;
    }
}

impl ToolDispatcher for AcpToolDispatcher {
    fn definitions(&self) -> Vec<ToolDef> {
        self.registry.definitions()
    }

    fn dispatch<'a>(&'a self, call: &'a ToolCall) -> BoxFuture<'a, ToolDispatchResult> {
        Box::pin(async move {
            let output = self
                .registry
                .execute(&call.name, &call.arguments, &self.context)
                .await;
            ToolDispatchResult {
                content: output.model_content(),
                parts: output.parts,
                is_error: output.is_error,
            }
        })
    }
}

struct AcpTodoStore {
    db_path: PathBuf,
}

impl tools::TodoStore for AcpTodoStore {
    fn list(&self, session_id: &str) -> Result<Vec<TodoItem>, String> {
        let db = Database::open(&self.db_path).map_err(|err| err.to_string())?;
        db.todos()
            .list_for_session(session_id)
            .map_err(|err| err.to_string())
    }

    fn replace(&self, session_id: &str, items: &[TodoItem]) -> Result<(), String> {
        let db = Database::open(&self.db_path).map_err(|err| err.to_string())?;
        db.todos()
            .replace_for_session(session_id, items)
            .map_err(|err| err.to_string())
    }
}

fn top_level_string(effective: &settings::EffectiveConfig, key: &str) -> Option<String> {
    effective
        .get(key)
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

fn provider_openrouter_string(effective: &settings::EffectiveConfig, key: &str) -> Option<String> {
    effective
        .get("provider")
        .and_then(|value| value.as_table())
        .and_then(|table| table.get("openrouter"))
        .and_then(|value| value.as_table())
        .and_then(|table| table.get(key))
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

fn openrouter_base_url_override(effective: &settings::EffectiveConfig) -> Option<String> {
    top_level_string(effective, "openrouter_base_url")
        .or_else(|| provider_openrouter_string(effective, "base_url"))
}

fn openrouter_api_key(effective: &settings::EffectiveConfig) -> Option<String> {
    provider_openrouter_string(effective, "api_key")
        .or_else(|| top_level_string(effective, "api_key"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::Duration;

    use futures_util::FutureExt;
    use futures_util::stream;
    use llm::{LlmEvent, LlmRequest, LlmStream, Usage};
    use tokio::time::{sleep, timeout};

    static TEST_ID: AtomicU64 = AtomicU64::new(0);

    #[derive(Clone, Default)]
    struct SharedBuffer {
        bytes: Arc<StdMutex<Vec<u8>>>,
    }

    impl Write for SharedBuffer {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.bytes
                .lock()
                .expect("buffer mutex")
                .extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl SharedBuffer {
        fn values(&self) -> Vec<Value> {
            let bytes = self.bytes.lock().expect("buffer mutex").clone();
            String::from_utf8(bytes)
                .expect("utf8")
                .lines()
                .map(|line| serde_json::from_str(line).expect("json line"))
                .collect()
        }
    }

    struct FakeProvider {
        chunks: Vec<String>,
        delay: Duration,
    }

    impl FakeProvider {
        fn new(chunks: &[&str]) -> Self {
            Self {
                chunks: chunks.iter().map(|chunk| (*chunk).to_owned()).collect(),
                delay: Duration::ZERO,
            }
        }

        fn delayed(chunks: Vec<String>, delay: Duration) -> Self {
            Self { chunks, delay }
        }
    }

    #[derive(Clone, Default)]
    struct ContextProvider {
        requests: Arc<StdMutex<Vec<LlmRequest>>>,
    }

    impl ContextProvider {
        fn requests(&self) -> Vec<LlmRequest> {
            self.requests.lock().expect("requests mutex").clone()
        }
    }

    impl Provider for ContextProvider {
        fn stream<'a>(
            &'a self,
            request: LlmRequest,
        ) -> BoxFuture<'a, Result<LlmStream, llm::LlmError>> {
            let requests = self.requests.clone();
            async move {
                let answer = context_answer(&request);
                requests.lock().expect("requests mutex").push(request);
                let events = vec![
                    LlmEvent::TextDelta { text: answer },
                    LlmEvent::Usage(Usage {
                        input_tokens: 2,
                        output_tokens: 2,
                        total_tokens: 4,
                        cost: Some(0.000002),
                    }),
                    LlmEvent::Done {
                        reason: llm::FinishReason::Stop,
                    },
                ];
                Ok(Box::pin(stream::iter(events)) as LlmStream)
            }
            .boxed()
        }
    }

    fn context_answer(request: &LlmRequest) -> String {
        let user_input = request
            .messages
            .last()
            .map(|message| message.content.as_str())
            .unwrap_or_default();
        if user_input.contains("Remember the word: marmalade") {
            return "acknowledged marmalade".to_owned();
        }
        if user_input.contains("What single word did I ask you to remember?") {
            return if request
                .messages
                .iter()
                .any(|message| message.content.contains("marmalade"))
            {
                "marmalade".to_owned()
            } else {
                "I do not know.".to_owned()
            };
        }
        if let Some(codeword) = user_input
            .strip_prefix("My codeword is ")
            .and_then(|rest| rest.split('.').next())
            .map(str::to_owned)
        {
            return format!("acknowledged {codeword}");
        }
        if user_input.contains("What is my codeword?") {
            return request
                .messages
                .iter()
                .find_map(|message| {
                    message
                        .content
                        .split_once("My codeword is ")
                        .and_then(|(_, rest)| rest.split('.').next())
                        .map(str::to_owned)
                })
                .unwrap_or_else(|| "I do not know any codeword.".to_owned());
        }
        "ok".to_owned()
    }

    impl Provider for FakeProvider {
        fn stream<'a>(
            &'a self,
            request: LlmRequest,
        ) -> BoxFuture<'a, Result<LlmStream, llm::LlmError>> {
            let _ = request;
            async move {
                let mut events = self
                    .chunks
                    .iter()
                    .cloned()
                    .map(|text| LlmEvent::TextDelta { text })
                    .collect::<Vec<_>>();
                events.push(LlmEvent::Usage(Usage {
                    input_tokens: 1,
                    output_tokens: 1,
                    total_tokens: 2,
                    cost: Some(0.000001),
                }));
                events.push(LlmEvent::Done {
                    reason: llm::FinishReason::Stop,
                });
                let delay = self.delay;
                Ok(Box::pin(stream::unfold(
                    (events.into_iter(), delay),
                    |(mut events, delay)| async move {
                        let next = events.next()?;
                        if !delay.is_zero() {
                            sleep(delay).await;
                        }
                        Some((next, (events, delay)))
                    },
                )) as LlmStream)
            }
            .boxed()
        }
    }

    fn temp_db_path(name: &str) -> PathBuf {
        let id = TEST_ID.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!("cortex-acp-{name}-{}-{id}.db", process::id()))
    }

    fn service_with_provider(provider: Arc<dyn Provider>) -> (AcpService, SharedBuffer) {
        let db_path = temp_db_path("unit");
        Database::open(&db_path).expect("db");
        let service = AcpService::with_provider(db_path, "google/gemini-2.5-flash", provider);
        (service, SharedBuffer::default())
    }

    fn request(id: i64, method: &str, params: Value) -> Value {
        json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        })
    }

    fn response_with_id(values: &[Value], id: i64) -> &Value {
        values
            .iter()
            .find(|value| value.get("id") == Some(&json!(id)))
            .expect("response id")
    }

    fn update_texts(values: &[Value]) -> Vec<String> {
        values
            .iter()
            .filter(|value| value.get("method") == Some(&json!("session/update")))
            .filter_map(|value| value.pointer("/params/update/content/text"))
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect()
    }

    fn initialize(service: &AcpService, writer: &JsonRpcWriter<SharedBuffer>) {
        service
            .handle_value(
                request(
                    1,
                    "initialize",
                    json!({
                        "protocolVersion": 1,
                        "clientCapabilities": {},
                        "clientInfo": { "name": "test", "version": "0" }
                    }),
                ),
                writer.clone(),
            )
            .expect("initialize");
    }

    fn new_session(service: &AcpService, writer: &JsonRpcWriter<SharedBuffer>) -> String {
        new_session_with_id(service, writer, 2)
    }

    fn new_session_with_id(
        service: &AcpService,
        writer: &JsonRpcWriter<SharedBuffer>,
        id: i64,
    ) -> String {
        service
            .handle_value(
                request(
                    id,
                    "session/new",
                    json!({
                        "cwd": std::env::current_dir().expect("cwd"),
                        "mcpServers": []
                    }),
                ),
                writer.clone(),
            )
            .expect("new session");
        writer
            .inner
            .lock()
            .expect("writer mutex")
            .values()
            .iter()
            .find(|value| value.get("id") == Some(&json!(id)))
            .and_then(|value| value.pointer("/result/sessionId"))
            .and_then(Value::as_str)
            .expect("session id")
            .to_owned()
    }

    async fn wait_for<F>(buffer: &SharedBuffer, mut predicate: F) -> Vec<Value>
    where
        F: FnMut(&[Value]) -> bool,
    {
        let result = timeout(Duration::from_secs(2), async {
            loop {
                let values = buffer.values();
                if predicate(&values) {
                    return values;
                }
                sleep(Duration::from_millis(10)).await;
            }
        })
        .await;
        result.expect("condition met")
    }

    #[tokio::test]
    async fn initialize_returns_protocol_and_capabilities() {
        let (service, buffer) = service_with_provider(Arc::new(FakeProvider::new(&[])));
        let writer = JsonRpcWriter::new(buffer.clone());

        initialize(&service, &writer);

        let values = buffer.values();
        let response = &values[0];
        assert_eq!(response["jsonrpc"], "2.0");
        assert_eq!(response["id"], 1);
        assert!(response.get("error").is_none());
        assert_eq!(response["result"]["protocolVersion"], 1);
        assert!(response["result"]["agentCapabilities"].is_object());
    }

    #[tokio::test]
    async fn malformed_json_error_keeps_loop_responsive() {
        let (service, buffer) = service_with_provider(Arc::new(FakeProvider::new(&[])));
        let writer = JsonRpcWriter::new(buffer.clone());
        let initialize = serde_json::to_string(&request(
            1,
            "initialize",
            json!({
                "protocolVersion": 1,
                "clientCapabilities": {},
                "clientInfo": { "name": "test", "version": "0" }
            }),
        ))
        .expect("serialize initialize");
        let input = format!("{{ this is not json\n{initialize}\n");

        service
            .run_loop(input.as_bytes(), writer)
            .await
            .expect("run loop");

        let values = buffer.values();
        assert_eq!(values[0]["jsonrpc"], "2.0");
        assert_eq!(values[0]["id"], Value::Null);
        assert_eq!(values[0]["error"]["code"], -32700);
        let initialize = response_with_id(&values, 1);
        assert!(initialize.get("error").is_none(), "{initialize}");
        assert_eq!(initialize["result"]["protocolVersion"], 1);
    }

    #[tokio::test]
    async fn structurally_invalid_request_returns_invalid_request_error() {
        let (service, buffer) = service_with_provider(Arc::new(FakeProvider::new(&[])));
        let writer = JsonRpcWriter::new(buffer.clone());

        service
            .handle_value(
                json!({ "jsonrpc": "1.0", "id": 99, "method": "initialize", "params": {} }),
                writer,
            )
            .expect("invalid request");

        let values = buffer.values();
        assert_eq!(values[0]["jsonrpc"], "2.0");
        assert_eq!(values[0]["id"], 99);
        assert_eq!(values[0]["error"]["code"], -32600);
    }

    #[tokio::test]
    async fn unknown_method_returns_method_not_found_with_matching_id() {
        let (service, buffer) = service_with_provider(Arc::new(FakeProvider::new(&[])));
        let writer = JsonRpcWriter::new(buffer.clone());
        initialize(&service, &writer);

        service
            .handle_value(request(9, "no/such/method", json!({})), writer)
            .expect("unknown method");

        let values = buffer.values();
        let response = response_with_id(&values, 9);
        assert_eq!(response["error"]["code"], -32601);
        assert!(
            response["error"]["message"]
                .as_str()
                .expect("message")
                .contains("method not found")
        );
    }

    #[tokio::test]
    async fn invalid_initialize_params_do_not_initialize_connection() {
        let (service, buffer) = service_with_provider(Arc::new(FakeProvider::new(&[])));
        let writer = JsonRpcWriter::new(buffer.clone());

        service
            .handle_value(request(1, "initialize", json!({})), writer.clone())
            .expect("invalid initialize");
        service
            .handle_value(
                request(
                    2,
                    "session/new",
                    json!({
                        "cwd": std::env::current_dir().expect("cwd"),
                        "mcpServers": []
                    }),
                ),
                writer,
            )
            .expect("session new before valid initialize");

        let values = buffer.values();
        assert_eq!(response_with_id(&values, 1)["error"]["code"], -32602);
        assert_eq!(response_with_id(&values, 2)["error"]["code"], -32002);
        assert!(!service.is_initialized());
    }

    #[tokio::test]
    async fn session_methods_before_initialize_are_rejected_without_updates() {
        let (service, buffer) = service_with_provider(Arc::new(FakeProvider::new(&[])));
        let writer = JsonRpcWriter::new(buffer.clone());

        service
            .handle_value(
                request(
                    2,
                    "session/new",
                    json!({
                        "cwd": std::env::current_dir().expect("cwd"),
                        "mcpServers": []
                    }),
                ),
                writer.clone(),
            )
            .expect("session new before initialize");
        service
            .handle_value(
                request(
                    3,
                    "session/prompt",
                    json!({ "sessionId": "ses_missing", "prompt": [{ "type": "text", "text": "hi" }] }),
                ),
                writer,
            )
            .expect("prompt before initialize");

        let values = buffer.values();
        assert_eq!(response_with_id(&values, 2)["error"]["code"], -32002);
        assert_eq!(response_with_id(&values, 3)["error"]["code"], -32002);
        assert!(update_texts(&values).is_empty());
    }

    #[tokio::test]
    async fn session_new_returns_non_empty_session_id() {
        let (service, buffer) = service_with_provider(Arc::new(FakeProvider::new(&[])));
        let writer = JsonRpcWriter::new(buffer.clone());
        initialize(&service, &writer);

        let session_id = new_session(&service, &writer);

        assert!(session_id.starts_with("ses_"));
    }

    #[tokio::test]
    async fn session_new_invalid_params_are_rejected() {
        let (service, buffer) = service_with_provider(Arc::new(FakeProvider::new(&[])));
        let writer = JsonRpcWriter::new(buffer.clone());
        initialize(&service, &writer);

        service
            .handle_value(request(2, "session/new", json!({})), writer)
            .expect("invalid session new");

        let values = buffer.values();
        assert_eq!(response_with_id(&values, 2)["error"]["code"], -32602);
    }

    #[tokio::test]
    async fn prompt_streams_agent_chunks_then_final_result() {
        let nonce = "nonce-acp-unit";
        let (service, buffer) =
            service_with_provider(Arc::new(FakeProvider::new(&["hello ", nonce])));
        let writer = JsonRpcWriter::new(buffer.clone());
        initialize(&service, &writer);
        let session_id = new_session(&service, &writer);

        service
            .handle_value(
                request(
                    3,
                    "session/prompt",
                    json!({
                        "sessionId": session_id,
                        "prompt": [{ "type": "text", "text": format!("echo {nonce}") }]
                    }),
                ),
                writer.clone(),
            )
            .expect("prompt");

        let values = wait_for(&buffer, |values| {
            values
                .iter()
                .any(|value| value.get("id") == Some(&json!(3)))
        })
        .await;
        let chunks = values
            .iter()
            .filter(|value| value.get("method") == Some(&json!("session/update")))
            .filter_map(|value| value.pointer("/params/update/content/text"))
            .filter_map(Value::as_str)
            .collect::<String>();
        assert!(chunks.contains(nonce), "{chunks:?}");
        let final_result = values
            .iter()
            .find(|value| value.get("id") == Some(&json!(3)))
            .expect("final result");
        assert_eq!(final_result["result"]["stopReason"], "end_turn");
        assert_eq!(final_result["result"]["queued"], false);
    }

    #[tokio::test]
    async fn overlapping_session_prompts_are_queued_and_report_queue_state() {
        let (service, buffer) = service_with_provider(Arc::new(FakeProvider::delayed(
            vec!["slow ".to_owned(), "response".to_owned()],
            Duration::from_millis(20),
        )));
        let writer = JsonRpcWriter::new(buffer.clone());
        initialize(&service, &writer);
        let session_id = new_session(&service, &writer);

        service
            .handle_value(
                request(
                    3,
                    "session/prompt",
                    json!({
                        "sessionId": session_id,
                        "prompt": [{ "type": "text", "text": "first queued prompt" }]
                    }),
                ),
                writer.clone(),
            )
            .expect("first prompt");

        tokio::time::sleep(Duration::from_millis(15)).await;
        service
            .handle_value(
                request(
                    4,
                    "session/prompt",
                    json!({
                        "sessionId": session_id,
                        "prompt": [{ "type": "text", "text": "second queued prompt" }]
                    }),
                ),
                writer.clone(),
            )
            .expect("second prompt");

        let values = wait_for(&buffer, |values| {
            values
                .iter()
                .any(|value| value.get("id") == Some(&json!(4)))
        })
        .await;
        let first = response_with_id(&values, 3);
        let second = response_with_id(&values, 4);
        assert_eq!(first["result"]["stopReason"], "end_turn");
        assert_eq!(first["result"]["queued"], false);
        assert_eq!(second["result"]["stopReason"], "end_turn");
        assert_eq!(second["result"]["queued"], true);
    }

    #[tokio::test]
    async fn prompts_in_one_session_retain_prior_context() {
        let provider = ContextProvider::default();
        let (service, buffer) = service_with_provider(Arc::new(provider.clone()));
        let writer = JsonRpcWriter::new(buffer.clone());
        initialize(&service, &writer);
        let session_id = new_session(&service, &writer);

        service
            .handle_value(
                request(
                    3,
                    "session/prompt",
                    json!({
                        "sessionId": session_id,
                        "prompt": [{ "type": "text", "text": "Remember the word: marmalade. Acknowledge." }]
                    }),
                ),
                writer.clone(),
            )
            .expect("first prompt");
        wait_for(&buffer, |values| {
            values
                .iter()
                .any(|value| value.get("id") == Some(&json!(3)))
        })
        .await;

        let start = buffer.values().len();
        service
            .handle_value(
                request(
                    4,
                    "session/prompt",
                    json!({
                        "sessionId": session_id,
                        "prompt": [{ "type": "text", "text": "What single word did I ask you to remember?" }]
                    }),
                ),
                writer,
            )
            .expect("follow-up prompt");
        let values = wait_for(&buffer, |values| {
            values
                .iter()
                .any(|value| value.get("id") == Some(&json!(4)))
        })
        .await;
        let follow_up_text = update_texts(&values[start..]).concat();
        assert!(
            follow_up_text.contains("marmalade"),
            "follow-up text: {follow_up_text:?}"
        );

        let requests = provider.requests();
        assert!(requests.len() >= 2);
        assert!(
            requests[1]
                .messages
                .iter()
                .any(|message| message.content.contains("marmalade")),
            "second provider request lacked prior session context: {:?}",
            requests[1].messages
        );
    }

    #[tokio::test]
    async fn repeated_sessions_are_isolated_on_one_process() {
        let provider = ContextProvider::default();
        let (service, buffer) = service_with_provider(Arc::new(provider.clone()));
        let writer = JsonRpcWriter::new(buffer.clone());
        initialize(&service, &writer);
        let session_a = new_session_with_id(&service, &writer, 2);
        let session_b = new_session_with_id(&service, &writer, 22);
        let codeword = "falcon-acp-isolated";

        service
            .handle_value(
                request(
                    3,
                    "session/prompt",
                    json!({
                        "sessionId": session_a,
                        "prompt": [{ "type": "text", "text": format!("My codeword is {codeword}. Acknowledge.") }]
                    }),
                ),
                writer.clone(),
            )
            .expect("session A prompt");
        wait_for(&buffer, |values| {
            values
                .iter()
                .any(|value| value.get("id") == Some(&json!(3)))
        })
        .await;

        let start_b = buffer.values().len();
        service
            .handle_value(
                request(
                    4,
                    "session/prompt",
                    json!({
                        "sessionId": session_b,
                        "prompt": [{ "type": "text", "text": "What is my codeword?" }]
                    }),
                ),
                writer.clone(),
            )
            .expect("session B prompt");
        let values = wait_for(&buffer, |values| {
            values
                .iter()
                .any(|value| value.get("id") == Some(&json!(4)))
        })
        .await;
        let session_b_text = update_texts(&values[start_b..]).concat();
        assert!(
            !session_b_text.contains(codeword),
            "session B leaked session A codeword: {session_b_text:?}"
        );
        assert!(
            session_b_text.contains("do not know"),
            "session B should report no context: {session_b_text:?}"
        );

        let start_a_follow_up = buffer.values().len();
        service
            .handle_value(
                request(
                    5,
                    "session/prompt",
                    json!({
                        "sessionId": session_a,
                        "prompt": [{ "type": "text", "text": "What is my codeword?" }]
                    }),
                ),
                writer,
            )
            .expect("session A follow-up");
        let values = wait_for(&buffer, |values| {
            values
                .iter()
                .any(|value| value.get("id") == Some(&json!(5)))
        })
        .await;
        let session_a_text = update_texts(&values[start_a_follow_up..]).concat();
        assert!(
            session_a_text.contains(codeword),
            "session A did not retain codeword: {session_a_text:?}"
        );

        let requests = provider.requests();
        assert!(requests.len() >= 3);
        assert!(
            !requests[1]
                .messages
                .iter()
                .any(|message| message.content.contains(codeword)),
            "session B provider request included session A history: {:?}",
            requests[1].messages
        );
        assert!(
            requests[2]
                .messages
                .iter()
                .any(|message| message.content.contains(codeword)),
            "session A follow-up request lacked session A history: {:?}",
            requests[2].messages
        );
    }

    #[tokio::test]
    async fn cancel_stops_in_flight_prompt_with_cancelled_result() {
        let chunks = (0..100).map(|idx| format!("{idx}\n")).collect::<Vec<_>>();
        let (service, buffer) = service_with_provider(Arc::new(FakeProvider::delayed(
            chunks,
            Duration::from_millis(25),
        )));
        let writer = JsonRpcWriter::new(buffer.clone());
        initialize(&service, &writer);
        let session_id = new_session(&service, &writer);

        service
            .handle_value(
                request(
                    3,
                    "session/prompt",
                    json!({
                        "sessionId": session_id,
                        "prompt": [{ "type": "text", "text": "count slowly" }]
                    }),
                ),
                writer.clone(),
            )
            .expect("prompt");
        wait_for(&buffer, |values| {
            values
                .iter()
                .any(|value| value.get("method") == Some(&json!("session/update")))
        })
        .await;

        service
            .handle_value(
                json!({
                    "jsonrpc": "2.0",
                    "method": "session/cancel",
                    "params": { "sessionId": session_id }
                }),
                writer.clone(),
            )
            .expect("cancel");
        let values = wait_for(&buffer, |values| {
            values
                .iter()
                .any(|value| value.pointer("/result/stopReason") == Some(&json!("cancelled")))
        })
        .await;
        let update_count = values
            .iter()
            .filter(|value| value.get("method") == Some(&json!("session/update")))
            .count();
        sleep(Duration::from_millis(100)).await;
        let later_count = buffer
            .values()
            .iter()
            .filter(|value| value.get("method") == Some(&json!("session/update")))
            .count();
        assert_eq!(later_count, update_count);
    }
}
