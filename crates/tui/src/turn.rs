//! Driving a single streaming turn from the TUI on a background thread.
//!
//! The interactive event loop is synchronous (it owns the terminal), so a turn runs on a dedicated
//! worker thread that builds the provider, assembles the [`Turn`], and drives the durable
//! [`SessionRunner`]. The runner's normalized [`RunnerEvent`] stream is forwarded over a channel as
//! [`TurnMessage`]s; the UI thread drains them each tick to grow the assistant text, surface
//! reasoning, update usage, and clear the busy indicator when the worker reports it is
//! [`TurnMessage::Finished`].
//!
//! No mocks live on this path: the same runner and OpenRouter provider the headless `cortex run`
//! uses drive the turn here. Setup failures (the network kill-switch, a missing credential, a bad
//! model id) are surfaced as a [`TurnMessage::SetupError`] so the UI renders a clear error state
//! instead of hanging.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::Duration;

use crate::permission::{PermissionAsk, UiResponder};
use crate::tooling::TurnDispatcher;
use engine::{
    Agent, CompactionPolicy, EnvironmentBlock, RunnerEvent, SessionRunner, SystemContext, Turn,
};
use events::Event;
use llm::Message;
use redaction::guard_outbound;
#[allow(dead_code)]
#[path = "../../cli/src/custom_agents.rs"]
mod custom_agents;

/// A message published by the turn worker for the UI thread to apply.
#[derive(Debug)]
pub enum TurnMessage {
    /// One normalized runner event (text/reasoning delta, usage, error, …).
    Event(RunnerEvent),
    /// A file changed by a tool this turn, rendered as a colored unified diff in the transcript.
    /// Boxed to keep the message enum small (a diff carries its full line list).
    FileDiff(Box<crate::diff::FileDiff>),
    /// An ask-gated tool is awaiting an interactive permission decision: the UI surfaces a prompt
    /// and answers on the carried reply channel, unblocking the worker. Boxed to keep the enum small.
    Permission(Box<PermissionAsk>),
    /// The turn could not start: a setup-time diagnostic (net-disabled, no-credential, bad model).
    SetupError(String),
    /// The worker has finished; the UI clears the busy indicator and finalizes the turn.
    Finished,
}

/// The outbound operation named in a `net-disabled` refusal raised before the turn starts.
const OUTBOUND_OPERATION: &str = "LLM turn";

/// Plain data needed to spawn one TUI turn worker.
pub struct SpawnRequest {
    pub prompt: String,
    pub user_parts: Vec<llm::MessageContentPart>,
    pub history: Vec<Message>,
    pub model: String,
    pub agent: String,
    pub variant: String,
    pub pure_plugins: bool,
    pub mcp_manager: Option<Arc<mcp::McpManager>>,
    pub remote: Option<RemoteBridge>,
    pub session_id: Option<String>,
    pub db_path: Option<PathBuf>,
    pub cancel: Arc<AtomicBool>,
}

/// Server bridge used when the TUI is attached to `cortex serve`.
#[derive(Clone, Debug)]
pub struct RemoteBridge {
    server: crate::remote::RemoteServer,
    session_id: String,
    message_id: String,
}

impl RemoteBridge {
    #[must_use]
    pub fn new(server: crate::remote::RemoteServer, session_id: String) -> Self {
        Self {
            server,
            session_id,
            message_id: remote_message_id(),
        }
    }

    fn publish(&self, event: Event) {
        let _ = self.server.publish_event(&event);
    }

    fn publish_started(&self) {
        self.publish(Event::turn_started(
            self.session_id.clone(),
            self.message_id.clone(),
        ));
    }

    fn publish_runner_event(&self, event: &RunnerEvent) {
        if let Some(event) = event_for_runner(&self.session_id, &self.message_id, event) {
            self.publish(event);
        }
    }

    fn publish_aborted(&self) {
        self.publish(Event::turn_done(
            self.session_id.clone(),
            self.message_id.clone(),
            "aborted",
            0,
        ));
    }
}

/// Spawn a worker thread that streams a turn for `prompt` (with prior `history` for context),
/// returning the receiver the UI drains for [`TurnMessage`]s.
///
/// The worker owns its own current-thread Tokio runtime and provider, so nothing but plain data
/// crosses the thread boundary. The receiver disconnects once the worker exits.
///
/// `cancel` is the interrupt seam: when the UI sets it (the cancel/interrupt key), the in-flight
/// runner future is dropped, which drops the upstream SSE stream and aborts the connection so the
/// turn stops streaming gracefully — no orphaned children, no panic. Background `bash` jobs are
/// still reaped and the checkpoint is still finalized afterward.
#[must_use]
pub fn spawn(request: SpawnRequest) -> Receiver<TurnMessage> {
    let (tx, rx) = mpsc::channel();
    let tx_events = tx.clone();
    thread::spawn(move || {
        let SpawnRequest {
            prompt,
            user_parts,
            history,
            model,
            agent,
            variant,
            pure_plugins,
            mcp_manager,
            remote,
            session_id,
            db_path,
            cancel,
        } = request;
        match build_turn(
            &prompt,
            user_parts,
            history,
            &model,
            &agent,
            &variant,
            pure_plugins,
        ) {
            Ok(setup) => {
                match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(runtime) => {
                        runtime.block_on(async {
                            // Materialize the per-turn tool set for the model's family, scoped to the
                            // process working directory, so the model can read/edit/create files for
                            // real this turn. The dispatcher emits a FileDiff for each change.
                            let cwd = std::env::current_dir()
                                .unwrap_or_else(|_| std::path::PathBuf::from("."));
                            let session_id = remote
                                .as_ref()
                                .map(|remote| remote.session_id.clone())
                                .or(session_id)
                                .unwrap_or_else(ephemeral_session_id);
                            let message_id = remote
                                .as_ref()
                                .map(|remote| remote.message_id.clone())
                                .unwrap_or_else(remote_message_id);
                            let audit_sink = Arc::new(
                                audit::AuditSink::from_env_or_cwd(&session_id, &cwd)
                                    .with_surface("tui"),
                            );
                            let lsp_hooks =
                                Arc::new(crate::lsp_bridge::LspHooks::new(cwd.clone()));
                            let mutation_hooks =
                                Arc::new(crate::format_bridge::PostMutationHooks::with_config(
                                    cwd.clone(),
                                    setup.formatter_config,
                                    lsp_hooks.clone(),
                                ));
                            // Open the shared shadow store and begin a turn recorder so each file
                            // this turn changes is snapshotted BEFORE it is written, powering on-disk
                            // `/undo` and `/redo`. Checkpointing degrades silently when the store
                            // cannot be opened — it never blocks the turn.
                            let recorder = match checkpoints::ShadowStore::open_default(&cwd) {
                                Ok(Some(store)) => {
                                    Some(Arc::new(store.begin_turn(session_id.clone())))
                                }
                                _ => None,
                            };
                            // Build the per-turn permission engine and its saved-decisions store
                            // (seeded from the shared on-disk store) so an interactive "always allow"
                            // is recorded and a previously saved decision is honored without
                            // re-asking. `loaded` is the on-disk baseline used to detect new saves.
                            let (engine, saved, loaded) =
                                build_permission_engine(tx_events.clone(), &cwd, &setup.agent);
                            let mut dispatcher = TurnDispatcher::new(
                                cwd.clone(),
                                &setup.model_id,
                                session_id.clone(),
                                tx_events.clone(),
                            )
                            .with_audit(audit_sink.clone())
                            .with_permissions(engine)
                            .with_hooks(mutation_hooks)
                            .with_lsp_service(lsp_hooks.clone())
                            .with_skills(setup.skills.clone());
                            if let Some(db_path) = db_path {
                                dispatcher = dispatcher.with_todo_store(Arc::new(
                                    crate::tooling::DurableTodos::new(db_path),
                                ));
                            }
                            if let Some(recorder) = &recorder {
                                dispatcher = dispatcher.with_checkpoints(Arc::new(
                                    crate::checkpoint::RecorderBridge::new(recorder.clone()),
                                ));
                            }
                            if let Some(manager) = &mcp_manager {
                                dispatcher = dispatcher.with_mcp(manager.clone());
                            }
                            let runner = SessionRunner::default();
                            if let Some(remote) = &remote {
                                remote.publish_started();
                            }
                            // Attach the shared audit sink so the TUI's plugin lifecycle/event-hook
                            // activity lands in the same sink as headless, attributed to "tui".
                            let plugin_hooks = setup.plugin_hooks.with_audit(audit_sink);
                            let started_event =
                                Event::turn_started(session_id.clone(), message_id.clone());
                            plugin_hooks.invoke_event(&started_event);
                            // Race the streaming turn against the interrupt signal: if the UI asks
                            // to cancel, the runner future is dropped here (closing the upstream
                            // stream) and the turn stops streaming at a safe boundary.
                            let cancelled = tokio::select! {
                                _ = runner.run_with_tools(setup.provider.as_ref(), setup.turn, &dispatcher, |event| {
                                    if let Some(remote) = &remote {
                                        remote.publish_runner_event(&event);
                                    }
                                    let _ = tx_events.send(TurnMessage::Event(event));
                                }) => false,
                                () = wait_for_cancel(cancel.clone()) => true,
                            };
                            if cancelled
                                && let Some(remote) = &remote
                            {
                                remote.publish_aborted();
                            }
                            if cancelled {
                                plugin_hooks.invoke_event(&Event::turn_done(
                                    session_id.clone(),
                                    message_id.clone(),
                                    "aborted",
                                    0,
                                ));
                            }
                            // Reap any background `bash` jobs spawned this turn by exact PID, so no
                            // orphan process outlives the turn (even after an interrupt).
                            dispatcher.background_jobs().shutdown().await;
                            // Likewise tear down any language server child the formatter/LSP hooks
                            // spawned for this turn.
                            lsp_hooks.shutdown();
                            // Finalize the checkpoint AFTER the turn: record each touched file's
                            // post-mutation state and append the snapshot. A read-only turn records
                            // nothing, so it creates no checkpoint churn; a failure degrades undo for
                            // this turn rather than disrupting the rendered session.
                            if let Some(recorder) = &recorder {
                                let _ = recorder.finalize();
                            }
                            // Persist any "always allow/deny" decision saved this turn to the shared
                            // per-project store, so a LATER run on this project — in EITHER surface,
                            // including a headless `cortex run` — honors it without re-asking
                            // (VAL-CROSS-028). Only a turn that recorded something new rewrites the
                            // store; a failure degrades to "not remembered" rather than disrupting
                            // the rendered session.
                            persist_saved_decisions(&cwd, &saved, &loaded);
                        });
                    }
                    Err(err) => {
                        let _ = tx.send(TurnMessage::SetupError(format!(
                            "failed to start runtime: {err}"
                        )));
                    }
                }
            }
            Err(message) => {
                let _ = tx.send(TurnMessage::SetupError(message));
            }
        }
        let _ = tx.send(TurnMessage::Finished);
    });
    rx
}

/// Resolve once the cancel flag is set, polling it on a short cadence so an interrupt is observed
/// promptly without busy-spinning. Used to race the streaming turn against the UI's interrupt key.
async fn wait_for_cancel(cancel: Arc<AtomicBool>) {
    while !cancel.load(Ordering::Relaxed) {
        tokio::time::sleep(Duration::from_millis(40)).await;
    }
}

fn event_for_runner(session_id: &str, message_id: &str, event: &RunnerEvent) -> Option<Event> {
    match event {
        RunnerEvent::TextDelta { text } => Some(Event::assistant_text_delta(
            session_id.to_owned(),
            message_id.to_owned(),
            text.clone(),
        )),
        RunnerEvent::Reasoning { text, .. } => Some(Event::assistant_reasoning_delta(
            session_id.to_owned(),
            message_id.to_owned(),
            text.clone(),
        )),
        RunnerEvent::ToolCall {
            id,
            name,
            arguments,
        } => Some(Event::tool_call_for_turn(
            session_id.to_owned(),
            message_id.to_owned(),
            id.clone(),
            name.clone(),
            arguments.clone(),
        )),
        RunnerEvent::ToolResult {
            id, name, is_error, ..
        } => Some(Event::tool_result_for_turn(
            session_id.to_owned(),
            message_id.to_owned(),
            id.clone(),
            name.clone(),
            *is_error,
        )),
        RunnerEvent::Usage(usage) => Some(Event::turn_usage(
            session_id.to_owned(),
            message_id.to_owned(),
            usage.input_tokens,
            usage.output_tokens,
            usage.total_tokens,
            usage.cache_creation_input_tokens,
            usage.cache_read_input_tokens,
            usage.cost,
        )),
        RunnerEvent::Error { message, kind } => Some(Event::turn_error(
            session_id.to_owned(),
            message_id.to_owned(),
            kind.clone(),
            message.clone(),
        )),
        RunnerEvent::Done { reason, steps } => Some(Event::turn_done(
            session_id.to_owned(),
            message_id.to_owned(),
            finish_reason_name(reason),
            *steps,
        )),
        RunnerEvent::ToolSetChanged { .. } => None,
        RunnerEvent::Retry { .. } => None,
    }
}

fn finish_reason_name(reason: &llm::FinishReason) -> String {
    serde_json::to_value(reason)
        .ok()
        .and_then(|value| value.as_str().map(ToOwned::to_owned))
        .unwrap_or_else(|| format!("{reason:?}"))
}

/// An ephemeral session id scoping this turn's tool context when the TUI cannot resolve a durable
/// session. Normal launches pass the persisted session id so tools share the on-disk store.
fn ephemeral_session_id() -> String {
    format!("tui-{}-{}", std::process::id(), monotonic_nanos())
}

fn remote_message_id() -> String {
    format!("tui-msg-{}-{}", std::process::id(), monotonic_nanos())
}

fn monotonic_nanos() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default()
}

/// Build the provider and assembled [`Turn`] for a prompt, running against the supplied active
/// model (the footer's model, which the `/models` picker updates) while resolving the credential
/// and any endpoint override through the same configuration the headless run uses.
///
/// # Errors
/// Returns a human-readable, secret-scrubbed message when the kill-switch refuses the turn, a
/// configuration layer cannot be loaded, the model id is malformed, or no credential is available.
struct TurnSetup {
    provider: Arc<dyn llm::Provider>,
    turn: Turn,
    model_id: String,
    agent: custom_agents::ActiveAgent,
    formatter_config: format::FormatterConfig,
    skills: Vec<skills::Skill>,
    plugin_hooks: plugins::PluginEventHooks,
}

fn build_turn(
    prompt: &str,
    user_parts: Vec<llm::MessageContentPart>,
    history: Vec<Message>,
    active_model: &str,
    active_agent: &str,
    active_variant: &str,
    pure_plugins: bool,
) -> Result<TurnSetup, String> {
    // Fail closed on the network kill-switch before resolving a credential or opening any socket.
    guard_outbound(OUTBOUND_OPERATION).map_err(|diag| diag.to_string())?;

    let inputs = settings::ConfigInputs::from_process(BTreeMap::new())
        .map_err(|err| format!("cannot determine the working directory: {err}"))?;
    let project_memory = memory::discover(&inputs);
    for warning in project_memory.warnings() {
        eprintln!("cortex tui: {warning}");
    }
    if project_memory.net_blocked() {
        return Err(
            "network is disabled by CORTEX_NO_NET=1; remote instruction URLs cannot be fetched"
                .to_owned(),
        );
    }
    let discovered_skills = skills::discover(&inputs);
    for warning in &discovered_skills.warnings {
        eprintln!("cortex tui: {warning}");
    }
    let discovered_skill_list = discovered_skills.ordered_skills();
    let discovered_agents = custom_agents::discover(&inputs);
    for warning in &discovered_agents.warnings {
        eprintln!("cortex tui: {warning}");
    }
    let plugin_cwd = inputs.cwd.clone();
    let effective = settings::load(inputs.clone()).map_err(|err| err.to_string())?;
    let formatter_config = format::FormatterConfig::from_table(effective.table())
        .map_err(|message| format!("invalid formatter config: {message}"))?;
    let provider_config = providers::ProviderConfigSet::from_table(effective.table())
        .map_err(|err| format!("invalid provider configuration: {err}"))?;
    let plugin_config =
        plugins::PluginConfig::from_toml_value(effective.get("plugin"), &plugin_cwd)
            .map_err(|message| format!("invalid plugin configuration: {message}"))?;
    let plugin_hooks =
        plugins::PluginEventHooks::load_or_disabled(&plugin_config, pure_plugins, "tui")
            .map_err(|err| format!("plugin runtime error: {err}"))?;
    let agent = custom_agents::resolve(non_empty(active_agent).as_deref(), &discovered_agents)
        .map_err(|err| {
            format!("{err}; select one with /agents or set a valid `agent` value in cortex.toml")
        })?;

    // Run against the active model the UI shows (set from config at launch, updated by `/models`),
    // so a per-session model choice governs the turn directly rather than via a config round-trip.
    let model = provider_config
        .resolve_model(providers::Catalogue::embedded(), Some(active_model))
        .map_err(|err| err.to_string())?;
    let protocol = provider_config.provider_protocol(&model.provider_id);
    validate_model_id(&model.model_id)?;
    validate_user_media_parts_for_model(&model, protocol, &user_parts)?;
    policy::enforce_provider_use(&inputs, &model.provider_id).map_err(|err| {
        if err.is_config() {
            format!("cannot load provider policy: {err}")
        } else {
            err.to_string()
        }
    })?;

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

    let compaction = compaction_policy(&model);
    let model_id = model.model_id;
    let history = sanitize_history_media_parts_for_protocol(history, protocol);
    let system = render_system_context(
        &model_id,
        &agent,
        project_memory.render(),
        &discovered_skill_list,
    );
    let mut turn = Turn::new(&model_id, system, prompt)
        .with_user_parts(user_parts)
        .with_history(history)
        .with_compaction(compaction);
    let mut request_params = provider_config.provider_params(&model.provider_id);
    let variant = active_variant.trim();
    if !variant.is_empty() && !variant.eq_ignore_ascii_case("default") {
        request_params.insert(
            "variant".to_owned(),
            serde_json::Value::String(variant.to_owned()),
        );
    }
    if !request_params.is_empty() {
        turn = turn.with_extra_params(request_params);
    }
    if let Some(temperature) = agent.temperature() {
        turn = turn.with_temperature(temperature);
    }

    Ok(TurnSetup {
        provider,
        turn,
        model_id,
        agent,
        formatter_config,
        skills: discovered_skill_list,
        plugin_hooks,
    })
}

fn validate_user_media_parts_for_model(
    model: &providers::ResolvedModel,
    protocol: providers::ProviderProtocol,
    user_parts: &[llm::MessageContentPart],
) -> Result<(), String> {
    if user_parts.is_empty() {
        return Ok(());
    }
    if model.info.as_ref().is_some_and(|info| !info.attachment) {
        return Err(format!(
            "Model `{}` does not support attachments. Remove attachments or switch to an attachment-capable model.",
            model.model_id
        ));
    }
    let Some(unsupported) = user_parts
        .iter()
        .find(|part| !media_part_supported_by_protocol(part, protocol))
    else {
        return Ok(());
    };
    Err(format!(
        "Unsupported attachment media `{}` for protocol `{}`. Remove the attachment or switch to a compatible provider/model.",
        media_part_label(unsupported),
        protocol.as_str()
    ))
}

fn sanitize_history_media_parts_for_protocol(
    mut history: Vec<Message>,
    protocol: providers::ProviderProtocol,
) -> Vec<Message> {
    for message in &mut history {
        if message.parts.is_empty() {
            continue;
        }
        message
            .parts
            .retain(|part| media_part_supported_by_protocol(part, protocol));
    }
    history
}

fn media_part_supported_by_protocol(
    part: &llm::MessageContentPart,
    protocol: providers::ProviderProtocol,
) -> bool {
    match part {
        llm::MessageContentPart::Reasoning { .. } => true,
        llm::MessageContentPart::Image { media_type, .. } => match protocol {
            providers::ProviderProtocol::OpenAiChat => {
                openai_chat_supported_image_media_type(media_type)
            }
            providers::ProviderProtocol::OpenAiResponses => false,
            providers::ProviderProtocol::AnthropicMessages
            | providers::ProviderProtocol::Gemini => true,
        },
        llm::MessageContentPart::Document { .. } | llm::MessageContentPart::File { .. } => {
            !matches!(
                protocol,
                providers::ProviderProtocol::OpenAiChat
                    | providers::ProviderProtocol::OpenAiResponses
            )
        }
    }
}

fn openai_chat_supported_image_media_type(media_type: &str) -> bool {
    matches!(
        media_type,
        "image/png" | "image/jpeg" | "image/gif" | "image/webp" | "image/bmp"
    )
}

fn media_part_label(part: &llm::MessageContentPart) -> String {
    match part {
        llm::MessageContentPart::Image { media_type, .. } => media_type.clone(),
        llm::MessageContentPart::Document { media_type, .. } => media_type.clone(),
        llm::MessageContentPart::File { media_type, .. } => media_type
            .clone()
            .unwrap_or_else(|| "application/octet-stream".to_owned()),
        llm::MessageContentPart::Reasoning { provider, .. } => format!("reasoning:{provider}"),
    }
}

fn render_system_context(
    model_id: &str,
    agent: &custom_agents::ActiveAgent,
    project_memory: Option<String>,
    skills: &[skills::Skill],
) -> String {
    let mut system = SystemContext::for_model(model_id, EnvironmentBlock::from_process())
        .with_project_memory(project_memory)
        .with_agent(agent.base_agent())
        .render();
    if let Some(prompt) = agent.custom_prompt() {
        system.push_str("\n\nCustom agent instructions:\n");
        system.push_str(prompt);
    }
    if let Some(hint) = skills::render_system_hint(skills) {
        system.push_str("\n\n");
        system.push_str(&hint);
    }
    system
}

/// Build the per-turn permission engine that gates the TUI's tools with an interactive prompt,
/// returning the engine, its shared saved-decisions store, and the on-disk baseline that store was
/// seeded from (so the caller can detect and persist a new "always" decision after the turn).
///
/// The base policy asks before any effectful tool (`write`/`edit`/`apply_patch`/`bash`) and allows
/// the read-only tools, so an ungoverned mutation surfaces a prompt rather than running silently;
/// the project's own `[[permission]]` config layers are applied on top (higher precedence). Project
/// trust denies are installed as hard denies so an untrusted project blocks mutating/exec tools
/// before saved decisions are consulted, identically to headless (VAL-CROSS-049). The engine consults the shared
/// saved-decisions store (seeded from disk) BEFORE the ruleset, so a decision saved earlier — in
/// this or another surface — is honored without re-asking (VAL-CROSS-028). The [`UiResponder`]
/// answers each remaining `ask` by forwarding it to the UI thread over `events` and blocking on the
/// human's decision (fail-closed if the UI is gone).
fn build_permission_engine(
    events: Sender<TurnMessage>,
    cwd: &std::path::Path,
    agent: &custom_agents::ActiveAgent,
) -> (
    Arc<permissions::PermissionEngine>,
    Arc<permissions::SavedDecisions>,
    Vec<permissions::SavedDecision>,
) {
    let base_agent = agent.base_agent();
    let base = default_ruleset();
    let inputs = settings::ConfigInputs::from_process(BTreeMap::new())
        .ok()
        .map(|mut inputs| {
            inputs.cwd = cwd.to_path_buf();
            inputs
        });
    let config = inputs
        .as_ref()
        .and_then(|inputs| permissions::load_agent_rulesets(inputs).ok())
        .map(|rulesets| rulesets.for_agent(agent.name()));
    let trust = inputs
        .as_ref()
        .and_then(|inputs| policy::resolve_trust(inputs).ok())
        .unwrap_or_default();
    let mut trust_overlay = permissions::Ruleset::empty();
    let mut trust_hard_denies = Vec::new();
    for rule in policy::trust_ruleset(trust).rules() {
        if rule.action == permissions::Action::Deny {
            trust_hard_denies.push(rule.clone());
        } else {
            trust_overlay.push(rule.clone());
        }
    }
    let hard_denies = readonly_agent_hard_denies(base_agent);
    let custom_permissions = permissions::Ruleset::new(agent.permissions().to_vec());
    let mode_layer = permissions::Ruleset::new(hard_denies.clone());
    let ruleset = match config {
        Some(config) => permissions::Ruleset::from_layers([
            base,
            config,
            custom_permissions,
            mode_layer,
            trust_overlay.clone(),
        ]),
        None => {
            permissions::Ruleset::from_layers([base, custom_permissions, mode_layer, trust_overlay])
        }
    };
    let mut effective_hard_denies = hard_denies;
    effective_hard_denies.extend(trust_hard_denies);

    // Seed the saved-decisions store from the shared per-project store on disk, keyed by the same
    // workspace key the checkpoint store uses, so the TUI and headless `cortex run` share one store.
    let loaded = permissions::store::load_for(&checkpoints::workspace_key(cwd));
    let saved = Arc::new(permissions::SavedDecisions::from_decisions(loaded.clone()));

    let responder: Arc<dyn permissions::Responder> = Arc::new(UiResponder::new(events));
    let engine = Arc::new(
        permissions::PermissionEngine::new(ruleset)
            .with_responder(responder)
            .with_hard_denies(permissions::Ruleset::new(effective_hard_denies))
            .with_saved_decisions(saved.clone()),
    );
    (engine, saved, loaded)
}

fn readonly_agent_hard_denies(agent: Agent) -> Vec<permissions::Rule> {
    match agent {
        Agent::Plan | Agent::Explore => ["write", "edit", "apply_patch", "format", "bash"]
            .into_iter()
            .map(|tool| permissions::Rule::new(tool, "*", permissions::Action::Deny))
            .collect(),
        Agent::Build | Agent::General => Vec::new(),
    }
}

/// Persist any "always allow/deny" decisions recorded this turn to the shared per-project store, so
/// a later run on this project — in either surface — honors them without re-asking (VAL-CROSS-028).
///
/// The store is rewritten only when the turn's snapshot differs from the on-disk `loaded` baseline
/// (i.e. a new decision was saved), so a turn that records nothing leaves the store untouched and a
/// read-only/declined turn never clobbers a previously saved decision. A write failure is ignored
/// (the decision is simply not remembered), never disrupting the rendered session.
fn persist_saved_decisions(
    cwd: &std::path::Path,
    saved: &Arc<permissions::SavedDecisions>,
    loaded: &[permissions::SavedDecision],
) {
    let snapshot = saved.snapshot();
    if snapshot.as_slice() != loaded {
        let _ = permissions::store::save_for(&checkpoints::workspace_key(cwd), &snapshot);
    }
}

/// The TUI's default permission policy starts from the cross-surface baseline (allow by default,
/// ask for external-directory access, and ask for dotenv reads except `.env.example`), then asks
/// before each effectful mutating tool.
fn default_ruleset() -> permissions::Ruleset {
    use permissions::{Action, Rule};
    let mut rules = permissions::permissive_default_rules();
    rules.extend([
        Rule::new("write", "*", Action::Ask),
        Rule::new("edit", "*", Action::Ask),
        Rule::new("apply_patch", "*", Action::Ask),
        Rule::new("bash", "*", Action::Ask),
    ]);
    permissions::Ruleset::new(rules)
}

/// Validate that a resolved model id is a fully-qualified `provider/model` OpenRouter id, rejecting
/// a bare id locally with an actionable message (mirroring the headless run's validation).
fn validate_model_id(model_id: &str) -> Result<(), String> {
    providers::validate_model_id(model_id)
}

/// The compaction policy derived from the resolved model's context window (an unknown model falls
/// back to the engine's default window), so an over-large turn is pruned rather than crashing.
fn compaction_policy(model: &providers::ResolvedModel) -> CompactionPolicy {
    let (context, output) = match &model.info {
        Some(info) => (
            usize::try_from(info.limit.context).unwrap_or(usize::MAX),
            usize::try_from(info.limit.output).unwrap_or(0),
        ),
        None => (engine::DEFAULT_CONTEXT_LIMIT_TOKENS, 0),
    };
    CompactionPolicy::for_context_window(context, output)
}

/// Trim a candidate value and keep it only if non-empty after trimming.
fn non_empty(value: &str) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU32, Ordering};

    struct Scratch {
        root: PathBuf,
        project: PathBuf,
    }

    impl Scratch {
        fn new(tag: &str) -> Self {
            static COUNTER: AtomicU32 = AtomicU32::new(0);
            let root = std::env::temp_dir().join(format!(
                "cortex-tui-turn-{tag}-{}-{}",
                std::process::id(),
                COUNTER.fetch_add(1, Ordering::Relaxed)
            ));
            let project = root.join("project");
            fs::create_dir_all(&project).expect("create project dir");
            Self { root, project }
        }

        fn write_project_config(&self, contents: &str) {
            fs::write(self.project.join("cortex.toml"), contents).expect("write project config");
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn bare_model_id_is_rejected() {
        let err = validate_model_id("not-a-real-model").unwrap_err();
        assert!(err.contains("provider/model"), "message: {err}");
    }

    #[test]
    fn qualified_model_id_passes() {
        assert!(validate_model_id("google/gemini-2.5-flash").is_ok());
    }

    #[test]
    fn empty_provider_or_model_half_is_rejected() {
        assert!(validate_model_id("/flash").is_err());
        assert!(validate_model_id("google/").is_err());
    }

    #[test]
    fn openai_chat_rejects_pdf_user_attachments_before_turn_start() {
        let model = providers::ResolvedModel {
            provider_id: "openrouter".to_owned(),
            model_id: "google/gemini-2.5-flash".to_owned(),
            info: Some(providers::ModelInfo {
                id: "google/gemini-2.5-flash".to_owned(),
                name: "Gemini".to_owned(),
                attachment: true,
                reasoning: false,
                tool_call: true,
                limit: providers::ModelLimit::default(),
                cost: providers::ModelCost::default(),
            }),
        };
        let parts = vec![llm::MessageContentPart::Document {
            media_type: "application/pdf".to_owned(),
            path: "/tmp/example.pdf".to_owned(),
        }];
        let err = validate_user_media_parts_for_model(
            &model,
            providers::ProviderProtocol::OpenAiChat,
            &parts,
        )
        .expect_err("pdf should be rejected on openai-chat");
        assert!(
            err.contains("application/pdf") && err.contains("openai-chat"),
            "error: {err}"
        );
    }

    #[test]
    fn history_media_sanitizer_drops_unsupported_openai_chat_parts() {
        let history = vec![Message::user("describe attachments").with_parts(vec![
            llm::MessageContentPart::Image {
                media_type: "image/png".to_owned(),
                path: "/tmp/a.png".to_owned(),
            },
            llm::MessageContentPart::Image {
                media_type: "image/svg+xml".to_owned(),
                path: "/tmp/a.svg".to_owned(),
            },
            llm::MessageContentPart::Document {
                media_type: "application/pdf".to_owned(),
                path: "/tmp/a.pdf".to_owned(),
            },
        ])];
        let sanitized = sanitize_history_media_parts_for_protocol(
            history,
            providers::ProviderProtocol::OpenAiChat,
        );
        assert_eq!(sanitized.len(), 1);
        assert_eq!(
            sanitized[0].parts,
            vec![llm::MessageContentPart::Image {
                media_type: "image/png".to_owned(),
                path: "/tmp/a.png".to_owned(),
            }]
        );
    }

    // The default policy asks before each effectful tool and allows the read-only tools, so an
    // ungoverned write/edit/apply_patch/bash surfaces a prompt while read/grep/glob run silently.
    #[test]
    fn default_policy_asks_for_effectful_tools_and_allows_reads() {
        use permissions::{Action, PermissionEngine};

        let engine = PermissionEngine::new(default_ruleset());
        for tool in ["write", "edit", "apply_patch", "bash"] {
            assert_eq!(
                engine.classify(tool, "anything"),
                Action::Ask,
                "{tool} must be ask-gated by default"
            );
        }
        for tool in ["read", "grep", "glob", "todo"] {
            assert_eq!(
                engine.classify(tool, "anything"),
                Action::Allow,
                "{tool} must be allowed by default"
            );
        }
    }

    #[test]
    fn system_context_advertises_discovered_skills_without_guidance() {
        let system = render_system_context(
            "google/gemini-2.5-flash",
            &custom_agents::ActiveAgent::Builtin(Agent::Build),
            None,
            &[skills::Skill {
                name: "cross-skill".to_owned(),
                description: "Use for cross-surface skill prompts".to_owned(),
                guidance: "Prefix every final answer with TUI-SKILL-SECRET.".to_owned(),
                source: std::path::PathBuf::from("SKILL.md"),
            }],
        );

        assert!(system.contains("Discovered skills are available"));
        assert!(system.contains("cross-skill"));
        assert!(system.contains("Use for cross-surface skill prompts"));
        assert!(
            !system.contains("TUI-SKILL-SECRET"),
            "skill bodies must remain inactive until the skill tool is called"
        );
    }

    #[test]
    fn system_context_uses_the_active_agent_persona() {
        let system = render_system_context(
            "google/gemini-2.5-flash",
            &custom_agents::ActiveAgent::Builtin(Agent::Plan),
            None,
            &[],
        );
        assert!(
            system.contains("plan persona") && system.contains("MUST NOT"),
            "active TUI agent should shape the system context: {system}"
        );
    }

    #[test]
    fn plan_agent_hard_denies_effectful_tools_before_prompting() {
        let (_tx, rx) = std::sync::mpsc::channel();
        let (engine, _saved, _loaded) = build_permission_engine(
            _tx,
            std::path::Path::new("."),
            &custom_agents::ActiveAgent::Builtin(Agent::Plan),
        );
        assert_eq!(
            engine.classify("bash", "echo hi"),
            permissions::Action::Deny,
            "plan mode should hard-deny bash rather than prompting"
        );
        assert!(
            rx.try_recv().is_err(),
            "a hard deny should not surface a permission prompt"
        );
    }

    #[test]
    fn untrusted_trust_deny_blocks_a_saved_allow_without_prompting() {
        let scratch = Scratch::new("trust-vs-saved");
        scratch.write_project_config("trust = \"untrusted\"\n");
        let workspace_key = checkpoints::workspace_key(&scratch.project);
        let store_path = permissions::store::store_path_with(
            &permissions::store::StoreInputs::from_process(),
            &workspace_key,
        )
        .expect("permission store path");

        permissions::store::write(
            &store_path,
            &[permissions::SavedDecision::new(
                "bash",
                "*",
                permissions::Decision::Allow,
            )],
        )
        .expect("seed saved decision");

        let (tx, rx) = std::sync::mpsc::channel();
        let (engine, _saved, _loaded) = build_permission_engine(
            tx,
            &scratch.project,
            &custom_agents::ActiveAgent::Builtin(Agent::Build),
        );

        assert_eq!(
            engine.classify("bash", "echo hi"),
            permissions::Action::Deny,
            "trust deny must outrank saved allow in untrusted projects"
        );
        assert_eq!(
            engine.evaluate(&permissions::PermissionRequest::new("bash", "echo hi")),
            permissions::Outcome::Denied
        );
        assert!(
            rx.try_recv().is_err(),
            "trust deny should block with no interactive ask"
        );
        let _ = fs::remove_file(store_path);
    }

    #[test]
    fn trusted_project_honors_saved_allow_without_reasking() {
        let scratch = Scratch::new("trusted-saved");
        scratch.write_project_config("trust = \"trusted\"\n");
        let workspace_key = checkpoints::workspace_key(&scratch.project);
        let store_path = permissions::store::store_path_with(
            &permissions::store::StoreInputs::from_process(),
            &workspace_key,
        )
        .expect("permission store path");

        permissions::store::write(
            &store_path,
            &[permissions::SavedDecision::new(
                "bash",
                "*",
                permissions::Decision::Allow,
            )],
        )
        .expect("seed saved decision");

        let (tx, rx) = std::sync::mpsc::channel();
        let (engine, _saved, _loaded) = build_permission_engine(
            tx,
            &scratch.project,
            &custom_agents::ActiveAgent::Builtin(Agent::Build),
        );

        assert_eq!(
            engine.classify("bash", "echo hi"),
            permissions::Action::Allow,
            "trusted contributes no trust deny, so saved allow remains effective"
        );
        assert_eq!(
            engine.evaluate(&permissions::PermissionRequest::new("bash", "echo hi")),
            permissions::Outcome::Allowed
        );
        assert!(
            rx.try_recv().is_err(),
            "saved allow should execute without surfacing an ask"
        );
        let _ = fs::remove_file(store_path);
    }

    #[test]
    fn system_context_includes_custom_agent_prompt() {
        let agent = custom_agents::parse_custom_agent(
            r#"---
name: sentinel
description: Sentinel
mode: primary
---
Always include SENTINEL-MARKER.
"#,
            std::path::PathBuf::from("sentinel.md"),
        )
        .expect("valid custom agent");
        let system = render_system_context(
            "google/gemini-2.5-flash",
            &custom_agents::ActiveAgent::Custom(agent),
            None,
            &[],
        );
        assert!(
            system.contains("Custom agent instructions") && system.contains("SENTINEL-MARKER"),
            "custom agent prompts should be injected into TUI system context: {system}"
        );
    }
}
