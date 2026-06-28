//! Agent Client Protocol (ACP) stdio surface.
//!
//! `cortex acp` is a thin JSON-RPC shell over the same durable runner used by the other surfaces:
//! it creates sessions in the shared SQLite store, runs real OpenRouter turns through
//! [`engine::SessionRunner`], streams assistant text as ACP `session/update` notifications, and
//! cancels in-flight turns by dropping the runner future at the stdio boundary.

use std::collections::{BTreeMap, HashMap};
use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::atomic::AtomicU64;
use std::sync::{Arc, Mutex as StdMutex};

use agent_client_protocol::schema::ProtocolVersion;
use agent_client_protocol::schema::v1::{
    AgentCapabilities, AuthMethod, AuthMethodAgent, AuthMethodId, CancelNotification, ContentBlock,
    ContentChunk, EmbeddedResourceResource, Implementation, InitializeRequest, InitializeResponse,
    MessageId, NewSessionRequest, NewSessionResponse, PromptCapabilities, PromptRequest,
    PromptResponse, SessionCapabilities, SessionCloseCapabilities, SessionDeleteCapabilities,
    SessionId, SessionListCapabilities, SessionNotification, SessionResumeCapabilities,
    SessionUpdate, StopReason, TextContent,
};
use db::{Database, DbPathInputs};
use engine::{
    CompactionPolicy, EnvironmentBlock, HistoryMessage, ModelFamily, PartLifecycleTracker,
    RunOutcome, RunnerEvent, SessionRunner, SystemContext, ToolDispatchResult, ToolDispatcher,
    Turn, TurnQueue, build_assistant_message_parts, extract_message_content_parts, project_history,
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

mod client;
mod sessions;

use client::{
    AcpPermissionResponder, AcpReadTool, AcpTerminalTool, AcpWriteTool, ClientConnection,
    OutboundSink,
};

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
    client_capabilities: StdMutex<ClientCapabilitiesSnapshot>,
    client_pending: StdMutex<HashMap<String, client::PendingReply>>,
    client_seq: AtomicU64,
    sessions: StdMutex<HashMap<String, SessionRuntime>>,
    credentials: StdMutex<HashMap<String, String>>,
}

/// The subset of client capabilities the ACP surface acts on, captured at `initialize`.
#[derive(Clone, Copy, Default)]
struct ClientCapabilitiesSnapshot {
    fs_read: bool,
    fs_write: bool,
    terminal: bool,
}

/// Per-session runtime state layered over the durable record: the active mode/agent, an in-memory
/// model override, and whether the session is open in this connection.
#[derive(Clone, Default)]
struct SessionRuntime {
    mode: Option<String>,
    model: Option<String>,
    open: bool,
}

impl AcpState {
    fn deliver_client_response(&self, id: &str, result: Result<Value, String>) {
        if let Some(pending) = self
            .client_pending
            .lock()
            .expect("client pending mutex")
            .remove(id)
        {
            pending.resolve(result);
        }
    }

    fn client_capabilities(&self) -> ClientCapabilitiesSnapshot {
        *self
            .client_capabilities
            .lock()
            .expect("client capabilities mutex")
    }

    fn session_runtime(&self, session_id: &str) -> SessionRuntime {
        self.sessions
            .lock()
            .expect("sessions mutex")
            .get(session_id)
            .cloned()
            .unwrap_or_default()
    }

    fn update_session_runtime(&self, session_id: &str, update: impl FnOnce(&mut SessionRuntime)) {
        let mut sessions = self.sessions.lock().expect("sessions mutex");
        update(sessions.entry(session_id.to_owned()).or_default());
    }

    fn credential_overrides(&self) -> HashMap<String, String> {
        self.credentials.lock().expect("credentials mutex").clone()
    }
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
                client_capabilities: StdMutex::new(ClientCapabilitiesSnapshot::default()),
                client_pending: StdMutex::new(HashMap::new()),
                client_seq: AtomicU64::new(0),
                sessions: StdMutex::new(HashMap::new()),
                credentials: StdMutex::new(HashMap::new()),
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
            if let Some(id) = object.get("id").and_then(Value::as_str) {
                let outcome = if let Some(error) = object.get("error") {
                    Err(error
                        .get("message")
                        .and_then(Value::as_str)
                        .map(str::to_owned)
                        .unwrap_or_else(|| error.to_string()))
                } else {
                    Ok(object.get("result").cloned().unwrap_or(Value::Null))
                };
                self.state.deliver_client_response(id, outcome);
            }
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
            "authenticate" => {
                if !self.is_initialized() {
                    return writer.send_error(id, -32002, "ACP connection is not initialized");
                }
                self.authenticate(id, params, writer)
            }
            "session/new" => {
                if !self.is_initialized() {
                    return writer.send_error(id, -32002, "ACP connection is not initialized");
                }
                self.new_session(id, params, writer)
            }
            "session/load" => {
                if !self.is_initialized() {
                    return writer.send_error(id, -32002, "ACP connection is not initialized");
                }
                self.load_session(id, params, writer)
            }
            "session/list" => {
                if !self.is_initialized() {
                    return writer.send_error(id, -32002, "ACP connection is not initialized");
                }
                self.list_sessions(id, params, writer)
            }
            "session/resume" => {
                if !self.is_initialized() {
                    return writer.send_error(id, -32002, "ACP connection is not initialized");
                }
                self.resume_session(id, params, writer)
            }
            "session/close" => {
                if !self.is_initialized() {
                    return writer.send_error(id, -32002, "ACP connection is not initialized");
                }
                self.close_session(id, params, writer)
            }
            "session/fork" => {
                if !self.is_initialized() {
                    return writer.send_error(id, -32002, "ACP connection is not initialized");
                }
                self.fork_session(id, params, writer)
            }
            "session/delete" => {
                if !self.is_initialized() {
                    return writer.send_error(id, -32002, "ACP connection is not initialized");
                }
                self.delete_session(id, params, writer)
            }
            "session/set_mode" => {
                if !self.is_initialized() {
                    return writer.send_error(id, -32002, "ACP connection is not initialized");
                }
                self.set_session_mode(id, params, writer)
            }
            "session/set_model" => {
                if !self.is_initialized() {
                    return writer.send_error(id, -32002, "ACP connection is not initialized");
                }
                self.set_session_model(id, params, writer)
            }
            "session/set_config_option" => {
                if !self.is_initialized() {
                    return writer.send_error(id, -32002, "ACP connection is not initialized");
                }
                self.set_session_config_option(id, params, writer)
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
        let client_caps = ClientCapabilitiesSnapshot {
            fs_read: request.client_capabilities.fs.read_text_file,
            fs_write: request.client_capabilities.fs.write_text_file,
            terminal: request.client_capabilities.terminal,
        };
        *self
            .state
            .client_capabilities
            .lock()
            .expect("client capabilities mutex") = client_caps;
        let capabilities = AgentCapabilities::new()
            .load_session(true)
            .prompt_capabilities(PromptCapabilities::new().embedded_context(true))
            .session_capabilities(
                SessionCapabilities::new()
                    .list(Some(SessionListCapabilities::new()))
                    .resume(Some(SessionResumeCapabilities::new()))
                    .close(Some(SessionCloseCapabilities::new()))
                    .delete(Some(SessionDeleteCapabilities::new())),
            );
        let response = InitializeResponse::new(protocol_version)
            .agent_capabilities(capabilities)
            .auth_methods(vec![AuthMethod::Agent(
                AuthMethodAgent::new(AuthMethodId::new("api-key"), "API key").description(Some(
                    "Provide a provider API key via authenticate".to_owned(),
                )),
            )])
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

        let runtime = self.state.session_runtime(&prompt.session_id);
        let agent = runtime
            .mode
            .as_deref()
            .and_then(|mode| engine::Agent::from_name(mode).ok())
            .unwrap_or(engine::Agent::Build);
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let plugin_cwd = cwd.clone();

        let sink: Arc<dyn OutboundSink> = Arc::new(writer.clone());
        let connection = ClientConnection::new(self.state.clone(), sink, prompt.session_id.clone());

        emit_prompt_start_updates(&writer, &prompt.session_id, agent.name());

        let system = SystemContext::for_model(&resolved.model_id, EnvironmentBlock::from_process())
            .with_agent(agent)
            .render();
        let mut turn = Turn::new(&resolved.model_id, system, prompt.text.clone())
            .with_history(history)
            .with_compaction(compaction_policy(&resolved.model));
        if !resolved.request_params.is_empty() {
            turn = turn.with_extra_params(resolved.request_params.clone());
        }
        let permissions = match permission_engine_for_turn(
            self.state.event_bus.clone(),
            &prompt.session_id,
            &user_message.id,
            agent,
            connection.clone(),
        ) {
            Ok(permissions) => permissions,
            Err(message) => {
                let _ = writer.send_error(id, -32000, message);
                return;
            }
        };
        let dispatcher = AcpToolDispatcher::for_model(
            cwd.clone(),
            &resolved.model_id,
            prompt.session_id.clone(),
            self.state.db_path.clone(),
            permissions,
            self.state.client_capabilities(),
            connection,
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
                cwd,
                db_path: self.state.db_path.clone(),
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
            mut outcome,
            runner_events,
        } = result;
        apply_estimated_usage_cost(&resolved.model, &mut outcome);

        if let Some(error) = &outcome.error {
            let _ = writer.send_error(id, -32000, error.to_string());
            return;
        }

        // Let plugins observe/transform compaction summaries before they are persisted, mirroring
        // the server and headless `run` paths so an ACP-driven compaction stays consistent across
        // surfaces. Loaded lazily only when the turn actually compacted.
        if !outcome.compactions.is_empty() {
            match load_plugin_compaction_hooks(&plugin_cwd) {
                Ok(hooks) => apply_plugin_compaction_hooks(
                    &hooks,
                    &resolved.model_id,
                    &mut outcome.compactions,
                ),
                Err(message) => {
                    let _ = writer.send_error(id, -32000, message);
                    return;
                }
            }
        }

        persist_usage(&db, &prompt.session_id, &outcome);
        let mut lifecycle = PartLifecycleTracker::new(&user_message.id, &resolved.model_id);
        for event in &runner_events {
            let _ = lifecycle.observe(event);
        }
        let part_timestamps = lifecycle.part_timestamps();
        let assistant_parts = build_assistant_message_parts(
            &resolved.model_id,
            agent.name(),
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
        let config_inputs = settings::ConfigInputs::from_process(BTreeMap::new())
            .map_err(|err| format!("cannot load configuration context: {err}"))?;
        let effective = settings::load(config_inputs.clone())
            .map_err(|err| format!("cannot load configuration: {err}"))?;
        let mut provider_config = providers::ProviderConfigSet::from_table(effective.table())
            .map_err(|err| format!("cannot parse provider configuration: {err}"))?;
        apply_credential_overrides(&mut provider_config, &self.state.credential_overrides());
        match &self.state.provider {
            #[cfg(test)]
            ProviderSource::Fixed { model_id, provider } => {
                let model = provider_config
                    .resolve_model(providers::Catalogue::embedded(), Some(model_id))
                    .map_err(|err| err.to_string())?;
                policy::enforce_provider_use(&config_inputs, &model.provider_id).map_err(
                    |err| {
                        if err.is_config() {
                            format!("cannot load provider policy: {err}")
                        } else {
                            err.to_string()
                        }
                    },
                )?;
                let request_params = provider_config.provider_params(&model.provider_id);
                Ok(ResolvedProvider {
                    model_id: model.model_id.clone(),
                    model,
                    provider: provider.clone(),
                    request_params,
                })
            }
            ProviderSource::OpenRouter => {
                let model_id = session_model
                    .map(str::trim)
                    .filter(|model| !model.is_empty())
                    .unwrap_or_else(|| effective.model());
                validate_model_id(model_id)?;
                let model = provider_config
                    .resolve_model(providers::Catalogue::embedded(), Some(model_id))
                    .map_err(|err| err.to_string())?;
                policy::enforce_provider_use(&config_inputs, &model.provider_id).map_err(
                    |err| {
                        if err.is_config() {
                            format!("cannot load provider policy: {err}")
                        } else {
                            err.to_string()
                        }
                    },
                )?;
                let provider = providers::resolve_runtime_provider_with_options(
                    &provider_config,
                    &model.provider_id,
                    &model.model_id,
                    None,
                )
                .map_err(|error| {
                    if let Some(config_error) = error.as_config() {
                        return config_error.to_string();
                    }
                    if let Some(diagnostic) = error.as_diagnostic() {
                        return diagnostic.to_string();
                    }
                    "failed to resolve runtime provider".to_owned()
                })?
                .provider;
                let request_params = provider_config.provider_params(&model.provider_id);
                Ok(ResolvedProvider {
                    model_id: model.model_id.clone(),
                    model,
                    provider,
                    request_params,
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
    request_params: serde_json::Map<String, serde_json::Value>,
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
    cwd: PathBuf,
    db_path: PathBuf,
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
    let cwd = events.cwd;
    let db_path = events.db_path;
    let captured_events: Arc<StdMutex<Vec<RunnerEvent>>> = Arc::new(StdMutex::new(Vec::new()));
    let captured_for_sink = captured_events.clone();
    tokio::select! {
        outcome = runner.run_with_tools(provider, turn, dispatcher, move |event| {
            publish_runner_event(&bus, &session_id, &message_id, &event);
            write_acp_update(&notify, &session_id, &message_id, &cwd, &db_path, &event);
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
    cwd: &std::path::Path,
    db_path: &std::path::Path,
    event: &RunnerEvent,
) where
    W: Write,
{
    match event {
        RunnerEvent::ToolCall {
            id,
            name,
            arguments,
        } => {
            emit_tool_call_update(writer, session_id, cwd, id, name, arguments);
            return;
        }
        RunnerEvent::ToolResult {
            id, name, is_error, ..
        } => {
            emit_tool_result_update(writer, session_id, db_path, id, name, *is_error);
            return;
        }
        _ => {}
    }
    let update = match event {
        RunnerEvent::TextDelta { text } if !text.is_empty() => {
            Some(SessionUpdate::AgentMessageChunk(
                ContentChunk::new(ContentBlock::Text(TextContent::new(text.clone())))
                    .message_id(Some(MessageId::new(message_id.to_owned()))),
            ))
        }
        RunnerEvent::Reasoning { text, .. } if !text.is_empty() => {
            Some(SessionUpdate::AgentThoughtChunk(
                ContentChunk::new(ContentBlock::Text(TextContent::new(text.clone())))
                    .message_id(Some(MessageId::new(format!("{message_id}-thought")))),
            ))
        }
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
                    "cache_creation_input_tokens": usage.cache_creation_input_tokens,
                    "cache_read_input_tokens": usage.cache_read_input_tokens,
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

fn send_session_update_json<W: Write>(writer: &JsonRpcWriter<W>, session_id: &str, update: Value) {
    let _ = writer.send_notification(
        "session/update",
        json!({ "sessionId": session_id, "update": update }),
    );
}

/// Emit the typed `tool_call` (and, for mutating tools, a `tool_call_update` carrying the file
/// diff) the ACP spec requires so clients render structured tool activity, not only thought text.
fn emit_tool_call_update<W: Write>(
    writer: &JsonRpcWriter<W>,
    session_id: &str,
    cwd: &std::path::Path,
    call_id: &str,
    name: &str,
    arguments: &str,
) {
    let parsed_args: Value = serde_json::from_str(arguments).unwrap_or(Value::Null);
    send_session_update_json(
        writer,
        session_id,
        json!({
            "sessionUpdate": "tool_call",
            "toolCallId": call_id,
            "title": tool_call_title(name, &parsed_args),
            "kind": client::tool_kind(name),
            "status": "pending",
            "rawInput": parsed_args,
        }),
    );
    if let Some(diff) = tool_diff(name, cwd, &parsed_args) {
        send_session_update_json(
            writer,
            session_id,
            json!({
                "sessionUpdate": "tool_call_update",
                "toolCallId": call_id,
                "status": "in_progress",
                "content": diff,
            }),
        );
    }
}

fn emit_tool_result_update<W: Write>(
    writer: &JsonRpcWriter<W>,
    session_id: &str,
    db_path: &std::path::Path,
    call_id: &str,
    name: &str,
    is_error: bool,
) {
    send_session_update_json(
        writer,
        session_id,
        json!({
            "sessionUpdate": "tool_call_update",
            "toolCallId": call_id,
            "status": if is_error { "failed" } else { "completed" },
        }),
    );
    if name == "todo"
        && let Some(plan) = plan_update_from_todos(db_path, session_id)
    {
        send_session_update_json(writer, session_id, plan);
    }
}

fn tool_call_title(name: &str, args: &Value) -> String {
    if let Some(path) = args.get("path").and_then(Value::as_str) {
        return format!("{name} {path}");
    }
    if let Some(command) = args.get("command").and_then(Value::as_str) {
        return format!("{name}: {command}");
    }
    name.to_owned()
}

/// Build the `diff` tool-call content block(s) from a mutating tool's arguments so the client can
/// render the proposed change. `write` carries the full new content; `edit` carries the old/new
/// strings; `apply_patch` (the GPT/Codex-family file-editing tool) carries one block per file
/// section in its patch envelope. Returns `None` for a non-mutating tool or unparseable arguments.
fn tool_diff(name: &str, cwd: &std::path::Path, args: &Value) -> Option<Vec<Value>> {
    let blocks = match name {
        "write" | "edit" => path_arg_diff(name, cwd, args).into_iter().collect(),
        "apply_patch" => apply_patch_diffs(cwd, args),
        _ => Vec::new(),
    };
    (!blocks.is_empty()).then_some(blocks)
}

/// Build a single `diff` content block for the path-argument tools (`write`/`edit`).
fn path_arg_diff(name: &str, cwd: &std::path::Path, args: &Value) -> Option<Value> {
    let path = args.get("path").and_then(Value::as_str)?;
    let absolute = if std::path::Path::new(path).is_absolute() {
        PathBuf::from(path)
    } else {
        cwd.join(path)
    };
    let path = absolute.to_string_lossy().to_string();
    match name {
        "write" => {
            let new_text = args.get("content").and_then(Value::as_str)?;
            Some(json!({ "type": "diff", "path": path, "newText": new_text }))
        }
        "edit" => {
            let new_text = args
                .get("new_string")
                .or_else(|| args.get("newText"))
                .and_then(Value::as_str)?;
            let old_text = args
                .get("old_string")
                .or_else(|| args.get("oldText"))
                .and_then(Value::as_str);
            let mut diff = json!({ "type": "diff", "path": path, "newText": new_text });
            if let Some(old_text) = old_text {
                diff["oldText"] = json!(old_text);
            }
            Some(diff)
        }
        _ => None,
    }
}

/// Build a `diff` content block per file section in an `apply_patch` envelope. GPT/Codex-family
/// models swap `write`/`edit` for `apply_patch`, so without this an ACP client driving such a model
/// would receive tool_call updates with no file diff block.
fn apply_patch_diffs(cwd: &std::path::Path, args: &Value) -> Vec<Value> {
    let Some(patch) = args
        .get("patch")
        .or_else(|| args.get("patchText"))
        .or_else(|| args.get("patch_text"))
        .or_else(|| args.get("input"))
        .and_then(Value::as_str)
    else {
        return Vec::new();
    };
    tools::patch_file_diffs(patch, cwd)
        .into_iter()
        .map(|diff| {
            let mut block = json!({
                "type": "diff",
                "path": diff.path.to_string_lossy(),
                "newText": diff.new_text,
            });
            if let Some(old_text) = diff.old_text {
                block["oldText"] = json!(old_text);
            }
            block
        })
        .collect()
}

fn plan_update_from_todos(db_path: &std::path::Path, session_id: &str) -> Option<Value> {
    let db = Database::open(db_path).ok()?;
    let todos = db.todos().list_for_session(session_id).ok()?;
    if todos.is_empty() {
        return None;
    }
    let entries = todos
        .iter()
        .map(|todo| {
            json!({
                "content": todo.content,
                "priority": "medium",
                "status": todo_status_name(&todo.status),
            })
        })
        .collect::<Vec<_>>();
    Some(json!({ "sessionUpdate": "plan", "entries": entries }))
}

fn todo_status_name(status: &protocol::TodoStatus) -> &'static str {
    match status {
        protocol::TodoStatus::Pending => "pending",
        protocol::TodoStatus::InProgress => "in_progress",
        // ACP plan entries have no cancelled state; report a settled task as completed.
        protocol::TodoStatus::Completed | protocol::TodoStatus::Cancelled => "completed",
    }
}

/// Emit the `available_commands_update` and `current_mode_update` at prompt start so the update
/// stream always advertises the slash-command catalogue and the active mode.
fn emit_prompt_start_updates<W: Write>(
    writer: &JsonRpcWriter<W>,
    session_id: &str,
    current_mode: &str,
) {
    let commands = [
        ("plan", "Switch to the read-only planning mode"),
        ("build", "Switch to the default build mode"),
        ("explore", "Switch to the read-only exploration mode"),
        ("general", "Switch to the general-purpose mode"),
    ]
    .iter()
    .map(|(name, description)| json!({ "name": name, "description": description }))
    .collect::<Vec<_>>();
    send_session_update_json(
        writer,
        session_id,
        json!({ "sessionUpdate": "available_commands_update", "availableCommands": commands }),
    );
    send_session_update_json(
        writer,
        session_id,
        json!({ "sessionUpdate": "current_mode_update", "currentModeId": current_mode }),
    );
}

fn publish_runner_event(bus: &EventBus, session_id: &str, message_id: &str, event: &RunnerEvent) {
    match event {
        RunnerEvent::TextDelta { text } => bus.publish(&Event::assistant_text_delta(
            session_id.to_owned(),
            message_id.to_owned(),
            text.clone(),
        )),
        RunnerEvent::Reasoning { text, .. } => bus.publish(&Event::assistant_reasoning_delta(
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
            usage.cache_creation_input_tokens,
            usage.cache_read_input_tokens,
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
                    let content_parts = extract_message_content_parts(&message.parts);
                    let content = if message.content.trim().is_empty() {
                        message
                            .parts
                            .iter()
                            .filter_map(|part| match &part.body {
                                protocol::MessagePartBody::Text { text }
                                | protocol::MessagePartBody::Reasoning { text, .. } => {
                                    Some(text.as_str())
                                }
                                _ => None,
                            })
                            .collect::<Vec<_>>()
                            .join("\n")
                    } else {
                        message.content.clone()
                    };
                    HistoryMessage::new(message.role, content).with_parts(content_parts)
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

fn apply_estimated_usage_cost(model: &ResolvedModel, outcome: &mut RunOutcome) {
    let Some(usage) = outcome.usage.as_mut() else {
        return;
    };
    if usage.cost.is_some() {
        return;
    }
    let Some(info) = model.info.as_ref() else {
        return;
    };
    usage.cost = providers::estimate_usage_cost(info, usage);
}

fn compaction_policy(model: &ResolvedModel) -> CompactionPolicy {
    // The effective context window can be overridden via `CORTEX_CONTEXT_LIMIT_TOKENS`, mirroring
    // the server and headless `run` paths so an ACP-driven session compacts consistently across
    // surfaces. An explicit override drives the budget purely from the requested window.
    if let Some(context) = context_limit_override() {
        return CompactionPolicy::for_context_window(context, 0);
    }
    let (context, output) = match &model.info {
        Some(info) => (
            usize::try_from(info.limit.context).unwrap_or(usize::MAX),
            usize::try_from(info.limit.output).unwrap_or(0),
        ),
        None => (engine::DEFAULT_CONTEXT_LIMIT_TOKENS, 0),
    };
    CompactionPolicy::for_context_window(context, output)
}

/// The operational override for the effective context window, read from
/// `CORTEX_CONTEXT_LIMIT_TOKENS` (matches the server and headless `run` knob).
fn context_limit_override() -> Option<usize> {
    parse_context_limit(std::env::var("CORTEX_CONTEXT_LIMIT_TOKENS").ok().as_deref())
}

/// Parse a context-window override token, accepting only a positive integer (trimmed). Returns
/// `None` for an absent, blank, zero, or unparseable value so the model-derived window stands.
fn parse_context_limit(raw: Option<&str>) -> Option<usize> {
    let parsed: usize = raw?.trim().parse().ok()?;
    (parsed >= 1).then_some(parsed)
}

/// Load plugin event hooks for the ACP surface so a compaction transformation matches the server
/// and headless `run` paths. Mirrors the server turn path (`load_or_disabled`): no configured
/// plugins yields an empty, no-op hook set without requiring a JS runtime.
fn load_plugin_compaction_hooks(
    cwd: &std::path::Path,
) -> Result<plugins::PluginEventHooks, String> {
    let config_inputs = settings::ConfigInputs::from_process(BTreeMap::new())
        .map_err(|err| format!("cannot load configuration context: {err}"))?;
    let effective =
        settings::load(config_inputs).map_err(|err| format!("cannot load configuration: {err}"))?;
    let plugin_config = plugins::PluginConfig::from_toml_value(effective.get("plugin"), cwd)
        .map_err(|message| format!("invalid plugin configuration: {message}"))?;
    plugins::PluginEventHooks::load_or_disabled(&plugin_config, false, "acp")
        .map_err(|err| format!("plugin runtime error: {err}"))
}

/// Let plugins observe/transform the turn's compaction records, converting through the engine's
/// hook-value seam so an ACP-driven compaction is transformed identically to the server/headless
/// paths.
fn apply_plugin_compaction_hooks(
    hooks: &plugins::PluginEventHooks,
    model_id: &str,
    compactions: &mut [engine::CompactionRecord],
) {
    if compactions.is_empty() {
        return;
    }
    let mut values: Vec<Value> = compactions
        .iter()
        .map(|compaction| compaction.to_hook_value(model_id))
        .collect();
    hooks.apply_compaction_hooks(&mut values);
    for (compaction, value) in compactions.iter_mut().zip(&values) {
        compaction.apply_hook_value(value);
    }
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
    agent: engine::Agent,
    connection: ClientConnection,
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
    let mut hard_denies = readonly_agent_restriction_rules(agent);
    for rule in policy::trust_ruleset(trust).rules() {
        if rule.action == permissions::Action::Deny {
            hard_denies.push(rule.clone());
        } else {
            ruleset.push(rule.clone());
        }
    }
    let responder = Arc::new(AcpPermissionResponder::new(connection));
    Ok(Some(Arc::new(
        PermissionEngine::new(ruleset)
            .with_hard_denies(permissions::Ruleset::new(hard_denies))
            .with_responder(responder)
            .with_bus(bus)
            .with_event_context(session_id.to_owned(), message_id.to_owned()),
    )))
}

/// Read-only mutation blocks for `plan`/`explore`, installed as hard denies so a saved "always
/// allow" cannot bypass them (mirrors the server/TUI agent-mode restriction invariant).
fn readonly_agent_restriction_rules(agent: engine::Agent) -> Vec<permissions::Rule> {
    match agent {
        engine::Agent::Plan | engine::Agent::Explore => {
            ["write", "edit", "apply_patch", "format", "bash"]
                .into_iter()
                .map(|tool| permissions::Rule::new(tool, "*", permissions::Action::Deny))
                .collect()
        }
        engine::Agent::Build | engine::Agent::General => Vec::new(),
    }
}

/// Inject client-supplied credentials (from `authenticate`) into the resolved provider config so a
/// later prompt resolves the provider without a separate CLI/env credential. An already-configured
/// or env credential still wins (the override is the lowest-precedence config value).
fn apply_credential_overrides(
    provider_config: &mut providers::ProviderConfigSet,
    overrides: &HashMap<String, String>,
) {
    for (provider_id, key) in overrides {
        let entry = provider_config
            .providers
            .entry(provider_id.clone())
            .or_insert_with(|| providers::ProviderRuntimeConfig {
                id: provider_id.clone(),
                enabled: true,
                ..Default::default()
            });
        if entry.auth.api_key.is_none() {
            entry.auth.api_key = Some(key.clone());
        }
    }
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
        client_capabilities: ClientCapabilitiesSnapshot,
        connection: ClientConnection,
    ) -> Self {
        let mut registry = match ModelFamily::from_model_id(model_id) {
            ModelFamily::Gpt | ModelFamily::Codex => ToolRegistry::gpt(),
            ModelFamily::Anthropic | ModelFamily::Gemini | ModelFamily::Default => {
                ToolRegistry::builtin()
            }
        };
        if client_capabilities.fs_read {
            registry.override_tool(Box::new(AcpReadTool::new(connection.clone())));
        }
        if client_capabilities.fs_write {
            registry.override_tool(Box::new(AcpWriteTool::new(connection.clone())));
        }
        if client_capabilities.terminal {
            registry.override_tool(Box::new(AcpTerminalTool::new(connection.clone())));
        }
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
            // Writers emit each JSON-RPC frame as payload bytes followed by a
            // trailing newline, so only bytes up to the last newline are a
            // fully-written frame. Ignore any trailing fragment to avoid
            // observing a concurrent writer mid-frame.
            let end = bytes
                .iter()
                .rposition(|byte| *byte == b'\n')
                .map_or(0, |index| index + 1);
            String::from_utf8(bytes[..end].to_vec())
                .expect("utf8")
                .lines()
                .filter(|line| !line.is_empty())
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

    #[derive(Clone, Default)]
    struct ReasoningReplayProvider {
        requests: Arc<StdMutex<Vec<LlmRequest>>>,
    }

    impl ReasoningReplayProvider {
        fn requests(&self) -> Vec<LlmRequest> {
            self.requests.lock().expect("requests mutex").clone()
        }
    }

    impl Provider for ReasoningReplayProvider {
        fn stream<'a>(
            &'a self,
            request: LlmRequest,
        ) -> BoxFuture<'a, Result<LlmStream, llm::LlmError>> {
            let requests = self.requests.clone();
            async move {
                let call = {
                    let mut guard = requests.lock().expect("requests mutex");
                    let call = guard.len();
                    guard.push(request);
                    call
                };
                let events = if call == 0 {
                    vec![
                        LlmEvent::Reasoning {
                            text: "reasoning for replay".to_owned(),
                            metadata: Some(serde_json::json!({
                                "provider": "anthropic",
                                "signature": "sig-acp-replay"
                            })),
                        },
                        LlmEvent::TextDelta {
                            text: "first answer".to_owned(),
                        },
                        LlmEvent::Usage(Usage {
                            input_tokens: 3,
                            output_tokens: 2,
                            total_tokens: 5,
                            cache_creation_input_tokens: None,
                            cache_read_input_tokens: None,
                            reasoning_output_tokens: None,
                            cost: Some(0.000002),
                        }),
                        LlmEvent::Done {
                            reason: llm::FinishReason::Stop,
                        },
                    ]
                } else {
                    vec![
                        LlmEvent::TextDelta {
                            text: "follow-up answer".to_owned(),
                        },
                        LlmEvent::Usage(Usage {
                            input_tokens: 4,
                            output_tokens: 2,
                            total_tokens: 6,
                            cache_creation_input_tokens: None,
                            cache_read_input_tokens: None,
                            reasoning_output_tokens: None,
                            cost: Some(0.000002),
                        }),
                        LlmEvent::Done {
                            reason: llm::FinishReason::Stop,
                        },
                    ]
                };
                Ok(Box::pin(stream::iter(events)) as LlmStream)
            }
            .boxed()
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
                        cache_creation_input_tokens: None,
                        cache_read_input_tokens: None,
                        reasoning_output_tokens: None,
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
                    cache_creation_input_tokens: None,
                    cache_read_input_tokens: None,
                    reasoning_output_tokens: None,
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
    async fn prompts_replay_reasoning_signature_parts_in_follow_up_history() {
        let provider = ReasoningReplayProvider::default();
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
                        "prompt": [{ "type": "text", "text": "first turn" }]
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

        service
            .handle_value(
                request(
                    4,
                    "session/prompt",
                    json!({
                        "sessionId": session_id,
                        "prompt": [{ "type": "text", "text": "second turn" }]
                    }),
                ),
                writer,
            )
            .expect("second prompt");
        wait_for(&buffer, |values| {
            values
                .iter()
                .any(|value| value.get("id") == Some(&json!(4)))
        })
        .await;

        let requests = provider.requests();
        assert!(requests.len() >= 2, "expected two provider requests");
        let replayed_reasoning = requests[1]
            .messages
            .iter()
            .flat_map(|message| message.parts.iter())
            .find_map(|part| match part {
                llm::MessageContentPart::Reasoning {
                    provider,
                    text,
                    signature,
                } => Some((provider.clone(), text.clone(), signature.clone())),
                _ => None,
            });
        assert_eq!(
            replayed_reasoning,
            Some((
                "anthropic".to_owned(),
                "reasoning for replay".to_owned(),
                Some("sig-acp-replay".to_owned())
            )),
            "second request should replay persisted reasoning metadata/signature parts"
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

    /// A provider that emits one round of tool calls and then a final text answer, so a turn
    /// exercises the full dispatch path (tool-call updates, client callbacks, permission gating).
    struct ToolCallingProvider {
        tool_calls: Vec<(String, String, Value)>,
        final_text: String,
        requests: Arc<StdMutex<Vec<LlmRequest>>>,
    }

    impl ToolCallingProvider {
        fn new(tool_calls: Vec<(&str, &str, Value)>, final_text: &str) -> Self {
            Self {
                tool_calls: tool_calls
                    .into_iter()
                    .map(|(id, name, args)| (id.to_owned(), name.to_owned(), args))
                    .collect(),
                final_text: final_text.to_owned(),
                requests: Arc::new(StdMutex::new(Vec::new())),
            }
        }
    }

    impl Provider for ToolCallingProvider {
        fn stream<'a>(
            &'a self,
            request: LlmRequest,
        ) -> BoxFuture<'a, Result<LlmStream, llm::LlmError>> {
            let tool_calls = self.tool_calls.clone();
            let final_text = self.final_text.clone();
            let requests = self.requests.clone();
            async move {
                let call_index = {
                    let mut guard = requests.lock().expect("requests mutex");
                    let index = guard.len();
                    guard.push(request);
                    index
                };
                let events = if call_index == 0 {
                    let mut events = tool_calls
                        .iter()
                        .map(|(id, name, args)| {
                            LlmEvent::ToolCall(llm::ToolCall {
                                id: id.clone(),
                                name: name.clone(),
                                arguments: args.to_string(),
                            })
                        })
                        .collect::<Vec<_>>();
                    events.push(LlmEvent::Done {
                        reason: llm::FinishReason::ToolCalls,
                    });
                    events
                } else {
                    vec![
                        LlmEvent::TextDelta {
                            text: final_text.clone(),
                        },
                        LlmEvent::Usage(Usage {
                            input_tokens: 1,
                            output_tokens: 1,
                            total_tokens: 2,
                            cache_creation_input_tokens: None,
                            cache_read_input_tokens: None,
                            reasoning_output_tokens: None,
                            cost: Some(0.000001),
                        }),
                        LlmEvent::Done {
                            reason: llm::FinishReason::Stop,
                        },
                    ]
                };
                Ok(Box::pin(stream::iter(events)) as LlmStream)
            }
            .boxed()
        }
    }

    fn initialize_with_caps(
        service: &AcpService,
        writer: &JsonRpcWriter<SharedBuffer>,
        capabilities: Value,
    ) {
        service
            .handle_value(
                request(
                    1,
                    "initialize",
                    json!({
                        "protocolVersion": 1,
                        "clientCapabilities": capabilities,
                        "clientInfo": { "name": "test", "version": "0" }
                    }),
                ),
                writer.clone(),
            )
            .expect("initialize");
    }

    fn send_prompt(
        service: &AcpService,
        writer: &JsonRpcWriter<SharedBuffer>,
        id: i64,
        session_id: &str,
        text: &str,
    ) {
        service
            .handle_value(
                request(
                    id,
                    "session/prompt",
                    json!({
                        "sessionId": session_id,
                        "prompt": [{ "type": "text", "text": text }]
                    }),
                ),
                writer.clone(),
            )
            .expect("prompt");
    }

    fn temp_dir(name: &str) -> PathBuf {
        let id = TEST_ID.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("cortex-acp-{name}-{}-{id}", process::id()));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    /// Wait for the first agent->client request of `method` at or after `after`, deliver `result`
    /// as the client's JSON-RPC response, and return the captured request.
    async fn answer_client_request(
        service: &AcpService,
        writer: &JsonRpcWriter<SharedBuffer>,
        buffer: &SharedBuffer,
        method: &str,
        after: usize,
        result: Value,
    ) -> Value {
        let values = wait_for(buffer, |values| {
            values.iter().skip(after).any(|value| {
                value.get("method") == Some(&json!(method))
                    && value
                        .get("id")
                        .and_then(Value::as_str)
                        .is_some_and(|id| id.starts_with("acp-srv-"))
            })
        })
        .await;
        let req = values
            .into_iter()
            .skip(after)
            .find(|value| value.get("method") == Some(&json!(method)))
            .expect("client request");
        let id = req.get("id").cloned().expect("client request id");
        service
            .handle_value(
                json!({ "jsonrpc": "2.0", "id": id, "result": result }),
                writer.clone(),
            )
            .expect("deliver client response");
        req
    }

    fn session_update_kinds(values: &[Value]) -> Vec<String> {
        values
            .iter()
            .filter(|value| value.get("method") == Some(&json!("session/update")))
            .filter_map(|value| value.pointer("/params/update/sessionUpdate"))
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect()
    }

    fn listed_session_ids(value: &Value) -> Vec<String> {
        value
            .pointer("/result/sessions")
            .and_then(Value::as_array)
            .expect("sessions array")
            .iter()
            .filter_map(|session| session.get("sessionId").and_then(Value::as_str))
            .map(str::to_owned)
            .collect()
    }

    // VAL-GAP-ACP-001
    #[tokio::test]
    async fn authenticate_stores_credential_and_advertises_method() {
        let (service, buffer) = service_with_provider(Arc::new(FakeProvider::new(&[])));
        let writer = JsonRpcWriter::new(buffer.clone());
        initialize(&service, &writer);

        let init = buffer.values()[0].clone();
        let auth_methods = init
            .pointer("/result/authMethods")
            .and_then(Value::as_array)
            .expect("authMethods advertised");
        assert!(
            auth_methods
                .iter()
                .any(|method| method.get("id").and_then(Value::as_str) == Some("api-key")),
            "initialize must advertise the api-key auth method: {auth_methods:?}"
        );

        let provided = "acp-fixture-value-001";
        service
            .handle_value(
                request(
                    7,
                    "authenticate",
                    json!({ "methodId": "api-key", "provider": "openrouter", "apiKey": provided }),
                ),
                writer.clone(),
            )
            .expect("authenticate");

        let values = buffer.values();
        let response = response_with_id(&values, 7);
        assert!(response.get("error").is_none(), "{response}");
        assert!(response.get("result").is_some());

        assert_eq!(
            service
                .state
                .credential_overrides()
                .get("openrouter")
                .map(String::as_str),
            Some(provided),
            "authenticate must store the credential for later prompts"
        );
        let on_the_wire = serde_json::to_string(&values).expect("serialize wire");
        assert!(
            !on_the_wire.contains(provided),
            "the API key must never be echoed back on the wire"
        );
    }

    // gap-hardening-3: the ACP context-limit override mirrors the server/headless parser.
    #[test]
    fn parse_context_limit_accepts_only_positive_integers() {
        assert_eq!(parse_context_limit(Some("4000")), Some(4_000));
        assert_eq!(parse_context_limit(Some("  16000 ")), Some(16_000));
        assert_eq!(parse_context_limit(None), None);
        assert_eq!(parse_context_limit(Some("")), None);
        assert_eq!(parse_context_limit(Some("0")), None);
        assert_eq!(parse_context_limit(Some("-5")), None);
        assert_eq!(parse_context_limit(Some("nope")), None);
    }

    // gap-hardening-3: tool_diff handles apply_patch so GPT/Codex-family ACP clients get the diff
    // content block, not only the gemini-path write/edit tools.
    #[test]
    fn tool_diff_builds_apply_patch_diff_blocks_for_add_and_update() {
        let dir = temp_dir("apply-patch-diff");

        // An Add File section: the diff carries the new content and no oldText.
        let add_patch = "*** Begin Patch\n*** Add File: added.txt\n+first\n+second\n*** End Patch";
        let blocks = tool_diff("apply_patch", &dir, &json!({ "patch": add_patch }))
            .expect("apply_patch produces diff blocks");
        assert_eq!(blocks.len(), 1, "one section -> one diff block: {blocks:?}");
        let add = &blocks[0];
        assert_eq!(add["type"], "diff");
        assert!(
            add["path"]
                .as_str()
                .is_some_and(|path| path.ends_with("added.txt")),
            "diff path resolves against cwd: {add:?}"
        );
        assert_eq!(add["newText"], "first\nsecond\n");
        assert!(add.get("oldText").is_none(), "an added file has no oldText");

        // An Update File section: the diff carries both the prior and patched content.
        std::fs::write(dir.join("app.py"), "value = 1\n").expect("seed file");
        let update_patch =
            "*** Begin Patch\n*** Update File: app.py\n@@\n-value = 1\n+value = 2\n*** End Patch";
        let blocks = tool_diff("apply_patch", &dir, &json!({ "patchText": update_patch }))
            .expect("apply_patch update produces diff blocks");
        assert_eq!(blocks.len(), 1);
        let update = &blocks[0];
        assert_eq!(update["oldText"], "value = 1\n");
        assert_eq!(update["newText"], "value = 2\n");

        // A non-mutating tool yields no diff blocks.
        assert!(tool_diff("read", &dir, &json!({ "path": "app.py" })).is_none());

        let _ = std::fs::remove_dir_all(&dir);
    }

    // gap-hardening-3: ACP compaction parity applies plugin compaction hooks like server/headless.
    #[test]
    fn apply_plugin_compaction_hooks_transforms_records_through_the_runtime() {
        if plugins::detect_runtime().is_none() {
            eprintln!("skipping ACP plugin compaction-hook proof: no Bun, Node, or Deno on PATH");
            return;
        }
        let dir = temp_dir("acp-compaction-plugin");
        let plugin = dir.join("plugin.mjs");
        std::fs::write(
            &plugin,
            r#"
export default async function plugin() {
  return {
    experimental: {
      session: {
        async compacting(record) {
          return { ...record, summary: `ACP_HOOKED:${record.summary}` };
        }
      }
    }
  };
}
"#,
        )
        .expect("write plugin");
        let config = plugins::PluginConfig::from_toml_value(
            toml::from_str::<toml::Value>("plugin = [\"./plugin.mjs\"]")
                .expect("toml")
                .get("plugin"),
            &dir,
        )
        .expect("plugin config");
        let hooks = plugins::PluginEventHooks::load(&config, "acp")
            .expect("plugin loads through the external runtime");

        let mut compactions = vec![engine::CompactionRecord {
            summary: "kept BLUEBIRD".to_owned(),
            tail_start_id: "tail-7".to_owned(),
            continued_request: true,
            model: None,
        }];
        apply_plugin_compaction_hooks(&hooks, "google/gemini-2.5-flash", &mut compactions);
        assert_eq!(compactions[0].summary, "ACP_HOOKED:kept BLUEBIRD");
        assert_eq!(compactions[0].tail_start_id, "tail-7");

        let _ = std::fs::remove_dir_all(&dir);
    }

    // VAL-GAP-ACP-001 (provider credential injection)
    #[test]
    fn apply_credential_overrides_injects_only_a_missing_api_key() {
        let mut config = providers::ProviderConfigSet::default();
        let mut overrides = HashMap::new();
        overrides.insert("openrouter".to_owned(), "injected-value".to_owned());
        apply_credential_overrides(&mut config, &overrides);
        assert_eq!(
            config
                .providers
                .get("openrouter")
                .and_then(|entry| entry.auth.api_key.as_deref()),
            Some("injected-value")
        );

        let mut later = HashMap::new();
        later.insert("openrouter".to_owned(), "other-value".to_owned());
        apply_credential_overrides(&mut config, &later);
        assert_eq!(
            config
                .providers
                .get("openrouter")
                .and_then(|entry| entry.auth.api_key.as_deref()),
            Some("injected-value"),
            "an existing credential must win over the authenticate override"
        );
    }

    // VAL-GAP-ACP-002
    #[tokio::test]
    async fn sessions_list_load_resume_close_fork_are_functional() {
        let answer = "ACP-SESSION-ANSWER";
        let (service, buffer) = service_with_provider(Arc::new(FakeProvider::new(&[answer])));
        let writer = JsonRpcWriter::new(buffer.clone());
        initialize(&service, &writer);
        let session_a = new_session_with_id(&service, &writer, 2);
        let session_b = new_session_with_id(&service, &writer, 3);

        let marker = "remember-acp-002";
        send_prompt(&service, &writer, 10, &session_a, &format!("note {marker}"));
        wait_for(&buffer, |values| {
            values
                .iter()
                .any(|value| value.get("id") == Some(&json!(10)))
        })
        .await;

        service
            .handle_value(request(20, "session/list", json!({})), writer.clone())
            .expect("list");
        let values = buffer.values();
        let ids = listed_session_ids(response_with_id(&values, 20));
        assert!(ids.contains(&session_a), "list missing session A: {ids:?}");
        assert!(ids.contains(&session_b), "list missing session B: {ids:?}");

        let start = buffer.values().len();
        service
            .handle_value(
                request(21, "session/load", json!({ "sessionId": session_a })),
                writer.clone(),
            )
            .expect("load");
        let values = wait_for(&buffer, |values| {
            values
                .iter()
                .any(|value| value.get("id") == Some(&json!(21)))
        })
        .await;
        let replay = update_texts(&values[start..]).concat();
        assert!(
            replay.contains(marker),
            "load did not replay user text: {replay:?}"
        );
        assert!(
            replay.contains(answer),
            "load did not replay agent text: {replay:?}"
        );
        assert!(
            response_with_id(&values, 21)
                .pointer("/result/modes/currentModeId")
                .is_some(),
            "load result must report the session mode state"
        );

        service
            .handle_value(
                request(22, "session/resume", json!({ "sessionId": session_a })),
                writer.clone(),
            )
            .expect("resume");
        let values = buffer.values();
        assert_eq!(
            response_with_id(&values, 22).pointer("/result/sessionId"),
            Some(&json!(session_a))
        );

        service
            .handle_value(
                request(23, "session/fork", json!({ "sessionId": session_a })),
                writer.clone(),
            )
            .expect("fork");
        let values = buffer.values();
        let child = response_with_id(&values, 23)
            .pointer("/result/sessionId")
            .and_then(Value::as_str)
            .expect("fork child id")
            .to_owned();
        assert_ne!(child, session_a, "fork must create a new session id");
        let start = buffer.values().len();
        service
            .handle_value(
                request(24, "session/load", json!({ "sessionId": child })),
                writer.clone(),
            )
            .expect("load child");
        let values = wait_for(&buffer, |values| {
            values
                .iter()
                .any(|value| value.get("id") == Some(&json!(24)))
        })
        .await;
        let child_replay = update_texts(&values[start..]).concat();
        assert!(
            child_replay.contains(marker),
            "fork child did not inherit the parent transcript: {child_replay:?}"
        );

        service
            .handle_value(
                request(25, "session/close", json!({ "sessionId": session_a })),
                writer.clone(),
            )
            .expect("close");
        let values = buffer.values();
        assert!(response_with_id(&values, 25).get("error").is_none());

        let db_path = service.state.db_path.clone();
        let restarted = AcpService::with_provider(
            db_path,
            "google/gemini-2.5-flash",
            Arc::new(FakeProvider::new(&[])),
        );
        let restart_buffer = SharedBuffer::default();
        let restart_writer = JsonRpcWriter::new(restart_buffer.clone());
        initialize(&restarted, &restart_writer);
        restarted
            .handle_value(
                request(30, "session/list", json!({})),
                restart_writer.clone(),
            )
            .expect("list after restart");
        let values = restart_buffer.values();
        let ids = listed_session_ids(response_with_id(&values, 30));
        assert!(
            ids.contains(&session_a),
            "session A lost across restart: {ids:?}"
        );
        assert!(
            ids.contains(&session_b),
            "session B lost across restart: {ids:?}"
        );
    }

    // VAL-GAP-ACP-003
    #[tokio::test]
    async fn mode_model_config_updates_emit_notifications_and_persist() {
        let (service, buffer) = service_with_provider(Arc::new(FakeProvider::new(&["ok"])));
        let writer = JsonRpcWriter::new(buffer.clone());
        initialize(&service, &writer);
        let session_id = new_session(&service, &writer);

        let start = buffer.values().len();
        service
            .handle_value(
                request(
                    10,
                    "session/set_mode",
                    json!({ "sessionId": session_id, "modeId": "plan" }),
                ),
                writer.clone(),
            )
            .expect("set_mode");
        let values = buffer.values();
        assert!(response_with_id(&values, 10).get("error").is_none());
        assert!(
            values[start..].iter().any(|value| {
                value.pointer("/params/update/sessionUpdate") == Some(&json!("current_mode_update"))
                    && value.pointer("/params/update/currentModeId") == Some(&json!("plan"))
            }),
            "set_mode must emit a current_mode_update naming the new mode"
        );
        assert_eq!(
            service.state.session_runtime(&session_id).mode.as_deref(),
            Some("plan")
        );

        let model = "anthropic/claude-3.7-sonnet";
        let start = buffer.values().len();
        service
            .handle_value(
                request(
                    11,
                    "session/set_model",
                    json!({ "sessionId": session_id, "model": model }),
                ),
                writer.clone(),
            )
            .expect("set_model");
        let values = buffer.values();
        assert!(response_with_id(&values, 11).get("error").is_none());
        let model_update = values[start..]
            .iter()
            .find(|value| {
                value.pointer("/params/update/sessionUpdate")
                    == Some(&json!("config_option_update"))
            })
            .expect("config_option_update");
        assert!(
            model_update.to_string().contains(model),
            "config update must name the new model: {model_update}"
        );
        let db = Database::open(&service.state.db_path).expect("db");
        assert_eq!(
            db.sessions()
                .require(&session_id)
                .expect("session")
                .model
                .as_deref(),
            Some(model),
            "set_model must persist to the durable store so later prompts use it"
        );

        let start = buffer.values().len();
        service
            .handle_value(
                request(
                    12,
                    "session/set_config_option",
                    json!({ "sessionId": session_id, "configId": "mode", "value": "explore" }),
                ),
                writer.clone(),
            )
            .expect("set_config_option");
        let values = buffer.values();
        assert!(response_with_id(&values, 12).get("error").is_none());
        assert!(values[start..].iter().any(|value| {
            value.pointer("/params/update/sessionUpdate") == Some(&json!("current_mode_update"))
                && value.pointer("/params/update/currentModeId") == Some(&json!("explore"))
        }));
        assert_eq!(
            service.state.session_runtime(&session_id).mode.as_deref(),
            Some("explore")
        );

        service
            .handle_value(
                request(
                    13,
                    "session/set_mode",
                    json!({ "sessionId": session_id, "modeId": "bogus" }),
                ),
                writer.clone(),
            )
            .expect("invalid set_mode");
        let values = buffer.values();
        assert_eq!(response_with_id(&values, 13)["error"]["code"], -32602);
    }

    // VAL-GAP-ACP-003 (mode alters the next prompt)
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn set_mode_plan_denies_write_in_subsequent_prompt() {
        let dir = temp_dir("planmode");
        let target = dir.join("should-not-exist.txt");
        let provider = ToolCallingProvider::new(
            vec![(
                "call-write",
                "write",
                json!({ "path": target.to_string_lossy(), "content": "blocked" }),
            )],
            "done",
        );
        let (service, buffer) = service_with_provider(Arc::new(provider));
        let writer = JsonRpcWriter::new(buffer.clone());
        initialize(&service, &writer);
        let session_id = new_session(&service, &writer);
        service
            .handle_value(
                request(
                    10,
                    "session/set_mode",
                    json!({ "sessionId": session_id, "modeId": "plan" }),
                ),
                writer.clone(),
            )
            .expect("set_mode");

        send_prompt(&service, &writer, 11, &session_id, "please write the file");
        let values = wait_for(&buffer, |values| {
            values
                .iter()
                .any(|value| value.get("id") == Some(&json!(11)))
        })
        .await;
        assert!(response_with_id(&values, 11).get("result").is_some());
        assert!(
            !target.exists(),
            "plan mode is a read-only hard deny; the write must have no disk side effect"
        );
        assert!(
            values.iter().any(|value| {
                value.pointer("/params/update/sessionUpdate") == Some(&json!("tool_call_update"))
                    && value.pointer("/params/update/status") == Some(&json!("failed"))
            }),
            "a denied tool call must surface a failed tool_call_update"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    // VAL-GAP-ACP-004
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn session_update_includes_tool_call_plan_diff_commands_and_mode() {
        let dir = temp_dir("updates");
        let write_target = dir.join("plan-output.txt");
        let provider = ToolCallingProvider::new(
            vec![
                (
                    "call-todo",
                    "todo",
                    json!({ "todos": [{ "content": "investigate", "status": "in_progress" }] }),
                ),
                (
                    "call-write",
                    "write",
                    json!({ "path": write_target.to_string_lossy(), "content": "result" }),
                ),
            ],
            "done",
        );
        let (service, buffer) = service_with_provider(Arc::new(provider));
        let writer = JsonRpcWriter::new(buffer.clone());
        initialize_with_caps(
            &service,
            &writer,
            json!({ "fs": { "writeTextFile": true } }),
        );
        let session_id = new_session(&service, &writer);
        send_prompt(&service, &writer, 11, &session_id, "plan then write");

        answer_client_request(
            &service,
            &writer,
            &buffer,
            "fs/write_text_file",
            0,
            json!({}),
        )
        .await;
        let values = wait_for(&buffer, |values| {
            values
                .iter()
                .any(|value| value.get("id") == Some(&json!(11)))
        })
        .await;
        assert!(response_with_id(&values, 11).get("result").is_some());

        let kinds = session_update_kinds(&values);
        for expected in [
            "tool_call",
            "tool_call_update",
            "plan",
            "available_commands_update",
            "current_mode_update",
        ] {
            assert!(
                kinds.iter().any(|kind| kind == expected),
                "missing `{expected}` session/update variant; saw {kinds:?}"
            );
        }
        assert!(
            values.iter().any(|value| {
                value.pointer("/params/update/sessionUpdate") == Some(&json!("tool_call"))
                    && value.pointer("/params/update/toolCallId").is_some()
            }),
            "tool_call updates must carry a toolCallId"
        );
        let has_diff = values.iter().any(|value| {
            value
                .pointer("/params/update/content")
                .and_then(Value::as_array)
                .is_some_and(|content| {
                    content
                        .iter()
                        .any(|block| block.get("type") == Some(&json!("diff")))
                })
        });
        assert!(
            has_diff,
            "tool_call_update must include a file diff content block"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    // gap-hardening-3: a GPT/Codex-family ACP client (which uses apply_patch instead of write/edit)
    // also receives the file diff content block on its tool_call updates.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn session_update_includes_apply_patch_diff_for_gpt_family_tools() {
        let patch = "*** Begin Patch\n*** Add File: gpt-output.txt\n+result line\n*** End Patch";
        let provider = ToolCallingProvider::new(
            vec![("call-apply", "apply_patch", json!({ "patch": patch }))],
            "done",
        );
        let (service, buffer) = service_with_provider(Arc::new(provider));
        let writer = JsonRpcWriter::new(buffer.clone());
        initialize(&service, &writer);
        let session_id = new_session(&service, &writer);
        send_prompt(&service, &writer, 11, &session_id, "patch the file");

        let values = wait_for(&buffer, |values| {
            values
                .iter()
                .any(|value| value.get("id") == Some(&json!(11)))
        })
        .await;
        assert!(response_with_id(&values, 11).get("result").is_some());

        let diff_block = values.iter().find_map(|value| {
            value
                .pointer("/params/update/content")
                .and_then(Value::as_array)
                .and_then(|content| {
                    content
                        .iter()
                        .find(|block| block.get("type") == Some(&json!("diff")))
                })
        });
        let diff_block = diff_block
            .expect("an apply_patch tool call must surface a diff content block for GPT clients");
        assert!(
            diff_block
                .get("path")
                .and_then(Value::as_str)
                .is_some_and(|path| path.ends_with("gpt-output.txt")),
            "diff block must name the patched file: {diff_block:?}"
        );
        assert_eq!(diff_block.get("newText"), Some(&json!("result line\n")));
    }

    // VAL-GAP-ACP-005
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn client_filesystem_callbacks_are_used_when_configured() {
        let dir = temp_dir("clientfs");
        let read_target = dir.join("client-read.txt");
        let write_target = dir.join("client-write.txt");
        let read_path = read_target.to_string_lossy().to_string();
        let write_path = write_target.to_string_lossy().to_string();
        let provider = ToolCallingProvider::new(
            vec![
                ("call-read", "read", json!({ "path": read_path })),
                (
                    "call-write",
                    "write",
                    json!({ "path": write_path, "content": "ROUTED-TO-CLIENT" }),
                ),
            ],
            "done",
        );
        let (service, buffer) = service_with_provider(Arc::new(provider));
        let writer = JsonRpcWriter::new(buffer.clone());
        initialize_with_caps(
            &service,
            &writer,
            json!({ "fs": { "readTextFile": true, "writeTextFile": true } }),
        );
        let session_id = new_session(&service, &writer);
        send_prompt(&service, &writer, 11, &session_id, "read then write");

        let read_req = answer_client_request(
            &service,
            &writer,
            &buffer,
            "fs/read_text_file",
            0,
            json!({ "content": "FROM-CLIENT" }),
        )
        .await;
        let write_req = answer_client_request(
            &service,
            &writer,
            &buffer,
            "fs/write_text_file",
            0,
            json!({}),
        )
        .await;

        let values = wait_for(&buffer, |values| {
            values
                .iter()
                .any(|value| value.get("id") == Some(&json!(11)))
        })
        .await;
        assert!(response_with_id(&values, 11).get("result").is_some());

        assert_eq!(
            read_req.pointer("/params/path").and_then(Value::as_str),
            Some(read_target.to_string_lossy().as_ref()),
            "client read callback must carry the resolved path"
        );
        assert_eq!(
            write_req.pointer("/params/path").and_then(Value::as_str),
            Some(write_target.to_string_lossy().as_ref())
        );
        assert_eq!(
            write_req.pointer("/params/content"),
            Some(&json!("ROUTED-TO-CLIENT"))
        );
        assert!(
            !write_target.exists(),
            "write must route to the client, leaving the local scratch disk unchanged"
        );
        assert!(
            !read_target.exists(),
            "read must route to the client, not touch the local scratch disk"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    // VAL-GAP-ACP-006
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn client_terminal_callbacks_execute_and_stream_output() {
        let provider = ToolCallingProvider::new(
            vec![("call-bash", "bash", json!({ "command": "echo hello-acp" }))],
            "done",
        );
        let (service, buffer) = service_with_provider(Arc::new(provider));
        let writer = JsonRpcWriter::new(buffer.clone());
        initialize_with_caps(&service, &writer, json!({ "terminal": true }));
        let session_id = new_session(&service, &writer);
        send_prompt(&service, &writer, 11, &session_id, "run echo");

        let create_req = answer_client_request(
            &service,
            &writer,
            &buffer,
            "terminal/create",
            0,
            json!({ "terminalId": "term-1" }),
        )
        .await;
        answer_client_request(
            &service,
            &writer,
            &buffer,
            "terminal/wait_for_exit",
            0,
            json!({ "exitCode": 0 }),
        )
        .await;
        answer_client_request(
            &service,
            &writer,
            &buffer,
            "terminal/output",
            0,
            json!({ "output": "hello-acp\n", "truncated": false }),
        )
        .await;
        let release_req =
            answer_client_request(&service, &writer, &buffer, "terminal/release", 0, json!({}))
                .await;

        let values = wait_for(&buffer, |values| {
            values
                .iter()
                .any(|value| value.get("id") == Some(&json!(11)))
        })
        .await;
        assert!(response_with_id(&values, 11).get("result").is_some());
        assert_eq!(
            create_req.pointer("/params/args"),
            Some(&json!(["-c", "echo hello-acp"])),
            "terminal/create must pass the command through the client shell"
        );
        assert_eq!(
            release_req
                .pointer("/params/terminalId")
                .and_then(Value::as_str),
            Some("term-1"),
            "the terminal must be released so no orphaned command remains"
        );
        assert!(
            values.iter().any(|value| {
                value.pointer("/params/update/sessionUpdate") == Some(&json!("tool_call_update"))
                    && value.pointer("/params/update/status") == Some(&json!("completed"))
            }),
            "a cleanly-exited terminal command must surface a completed tool_call_update"
        );
    }

    // VAL-GAP-ACP-007
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn permission_request_round_trips_through_client() {
        let (service, buffer) = service_with_provider(Arc::new(FakeProvider::new(&[])));
        let writer = JsonRpcWriter::new(buffer.clone());

        let allow_dir = temp_dir("perm-allow");
        let mut allow_rules = permissions::permissive_default_rules();
        allow_rules.push(permissions::Rule::new(
            "write",
            "*",
            permissions::Action::Ask,
        ));
        let allow_engine = PermissionEngine::new(permissions::Ruleset::new(allow_rules))
            .with_responder(Arc::new(AcpPermissionResponder::new(
                ClientConnection::new(
                    service.state.clone(),
                    Arc::new(writer.clone()),
                    "ses_perm_allow".to_owned(),
                ),
            )));
        let allow_ctx = ToolContext::new(allow_dir.clone())
            .with_permissions(Arc::new(allow_engine))
            .with_session("ses_perm_allow");
        let allow_args =
            serde_json::to_string(&json!({ "path": "allowed.txt", "content": "ok" })).unwrap();
        let allow_handle = tokio::spawn(async move {
            ToolRegistry::builtin()
                .execute("write", &allow_args, &allow_ctx)
                .await
        });
        let request_value = answer_client_request(
            &service,
            &writer,
            &buffer,
            "session/request_permission",
            0,
            json!({ "outcome": { "outcome": "selected", "optionId": "allow-once" } }),
        )
        .await;
        let allow_output = allow_handle.await.expect("join allow");
        assert!(!allow_output.is_error, "{}", allow_output.model_content());
        assert!(
            allow_dir.join("allowed.txt").exists(),
            "an allowed write must produce its file"
        );
        assert_eq!(
            request_value
                .pointer("/params/toolCall/rawInput/tool")
                .and_then(Value::as_str),
            Some("write"),
            "permission request must name the gated action"
        );
        assert!(
            request_value
                .pointer("/params/toolCall/locations/0/path")
                .and_then(Value::as_str)
                .is_some(),
            "permission request must include the target path"
        );

        let before_deny = buffer.values().len();
        let deny_dir = temp_dir("perm-deny");
        let mut deny_rules = permissions::permissive_default_rules();
        deny_rules.push(permissions::Rule::new(
            "write",
            "*",
            permissions::Action::Ask,
        ));
        let deny_engine = PermissionEngine::new(permissions::Ruleset::new(deny_rules))
            .with_responder(Arc::new(AcpPermissionResponder::new(
                ClientConnection::new(
                    service.state.clone(),
                    Arc::new(writer.clone()),
                    "ses_perm_deny".to_owned(),
                ),
            )));
        let deny_ctx = ToolContext::new(deny_dir.clone())
            .with_permissions(Arc::new(deny_engine))
            .with_session("ses_perm_deny");
        let deny_args =
            serde_json::to_string(&json!({ "path": "denied.txt", "content": "no" })).unwrap();
        let deny_handle = tokio::spawn(async move {
            ToolRegistry::builtin()
                .execute("write", &deny_args, &deny_ctx)
                .await
        });
        let feedback = "do not modify that file";
        answer_client_request(
            &service,
            &writer,
            &buffer,
            "session/request_permission",
            before_deny,
            json!({
                "outcome": {
                    "outcome": "selected",
                    "optionId": "reject-once",
                    "_meta": { "feedback": feedback }
                }
            }),
        )
        .await;
        let deny_output = deny_handle.await.expect("join deny");
        assert!(
            deny_output.is_error,
            "a rejected write must report an error"
        );
        assert!(
            deny_output.model_content().contains(feedback),
            "denied write must carry the client feedback: {}",
            deny_output.model_content()
        );
        assert!(
            !deny_dir.join("denied.txt").exists(),
            "a denied write must have no disk side effect"
        );

        let _ = std::fs::remove_dir_all(&allow_dir);
        let _ = std::fs::remove_dir_all(&deny_dir);
    }
}
