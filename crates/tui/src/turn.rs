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
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::Duration;

use engine::{
    Agent, CompactionPolicy, EnvironmentBlock, RunnerEvent, SessionRunner, SystemContext, Turn,
};
use events::Event;
use llm::Message;
use redaction::guard_outbound;
use settings::EffectiveConfig;

use crate::permission::{PermissionAsk, UiResponder};
use crate::tooling::TurnDispatcher;

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
pub fn spawn(
    prompt: String,
    history: Vec<Message>,
    model: String,
    mcp_manager: Option<Arc<mcp::McpManager>>,
    remote: Option<RemoteBridge>,
    cancel: Arc<AtomicBool>,
) -> Receiver<TurnMessage> {
    let (tx, rx) = mpsc::channel();
    let tx_events = tx.clone();
    thread::spawn(move || {
        match build_turn(&prompt, history, &model) {
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
                                .unwrap_or_else(ephemeral_session_id);
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
                                build_permission_engine(tx_events.clone(), &cwd);
                            let mut dispatcher = TurnDispatcher::new(
                                cwd.clone(),
                                &setup.model_id,
                                session_id,
                                tx_events.clone(),
                            )
                            .with_permissions(engine)
                            .with_hooks(mutation_hooks)
                            .with_lsp_service(lsp_hooks.clone())
                            .with_skills(setup.skills.clone());
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
                            // Race the streaming turn against the interrupt signal: if the UI asks
                            // to cancel, the runner future is dropped here (closing the upstream
                            // stream) and the turn stops streaming at a safe boundary.
                            let cancelled = tokio::select! {
                                _ = runner.run_with_tools(&setup.provider, setup.turn, &dispatcher, |event| {
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
        RunnerEvent::Reasoning { text } => Some(Event::assistant_reasoning_delta(
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
    }
}

fn finish_reason_name(reason: &llm::FinishReason) -> String {
    serde_json::to_value(reason)
        .ok()
        .and_then(|value| value.as_str().map(ToOwned::to_owned))
        .unwrap_or_else(|| format!("{reason:?}"))
}

/// An ephemeral session id scoping this turn's tool context (the TUI does not yet persist sessions;
/// the id only keys the in-memory per-session todo list for the turn).
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
    provider: llm::OpenAiCompatible,
    turn: Turn,
    model_id: String,
    formatter_config: format::FormatterConfig,
    skills: Vec<skills::Skill>,
}

fn build_turn(
    prompt: &str,
    history: Vec<Message>,
    active_model: &str,
) -> Result<TurnSetup, String> {
    // Fail closed on the network kill-switch before resolving a credential or opening any socket.
    guard_outbound(OUTBOUND_OPERATION).map_err(|diag| diag.to_string())?;

    let inputs = settings::ConfigInputs::from_process(BTreeMap::new())
        .map_err(|err| format!("cannot determine the working directory: {err}"))?;
    let project_memory = memory::discover(&inputs);
    for warning in project_memory.warnings() {
        eprintln!("cortex tui: {warning}");
    }
    let discovered_skills = skills::discover(&inputs);
    for warning in &discovered_skills.warnings {
        eprintln!("cortex tui: {warning}");
    }
    let discovered_skill_list = discovered_skills.ordered_skills();
    let effective = settings::load(inputs).map_err(|err| err.to_string())?;
    let formatter_config = format::FormatterConfig::from_table(effective.table())
        .map_err(|message| format!("invalid formatter config: {message}"))?;

    // Run against the active model the UI shows (set from config at launch, updated by `/models`),
    // so a per-session model choice governs the turn directly rather than via a config round-trip.
    let model = providers::resolve_model(Some(active_model));
    validate_model_id(&model.model_id)?;

    let credentials =
        providers::CredentialInputs::openrouter_from_process(openrouter_api_key(&effective));
    let provider = providers::resolve_openrouter_with_options(
        &credentials,
        openrouter_base_url_override(&effective).as_deref(),
        None,
    )
    .map_err(|diag| diag.to_string())?;

    let compaction = compaction_policy(&model);
    let model_id = model.model_id;
    let system = render_system_context(&model_id, project_memory.render(), &discovered_skill_list);
    let turn = Turn::new(&model_id, system, prompt)
        .with_history(history)
        .with_compaction(compaction);

    Ok(TurnSetup {
        provider,
        turn,
        model_id,
        formatter_config,
        skills: discovered_skill_list,
    })
}

fn render_system_context(
    model_id: &str,
    project_memory: Option<String>,
    skills: &[skills::Skill],
) -> String {
    let mut system = SystemContext::for_model(model_id, EnvironmentBlock::from_process())
        .with_project_memory(project_memory)
        .with_agent(Agent::default())
        .render();
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
/// the project's own `[[permission]]` config layers are applied on top (higher precedence), and the
/// project trust layer is applied last (highest precedence) so an untrusted project denies the
/// mutating/exec tools identically to headless (VAL-CROSS-030). The engine consults the shared
/// saved-decisions store (seeded from disk) BEFORE the ruleset, so a decision saved earlier — in
/// this or another surface — is honored without re-asking (VAL-CROSS-028). The [`UiResponder`]
/// answers each remaining `ask` by forwarding it to the UI thread over `events` and blocking on the
/// human's decision (fail-closed if the UI is gone).
fn build_permission_engine(
    events: Sender<TurnMessage>,
    cwd: &std::path::Path,
) -> (
    Arc<permissions::PermissionEngine>,
    Arc<permissions::SavedDecisions>,
    Vec<permissions::SavedDecision>,
) {
    let base = default_ruleset();
    let inputs = settings::ConfigInputs::from_process(BTreeMap::new()).ok();
    let config = inputs
        .as_ref()
        .and_then(|inputs| permissions::load_layered(inputs).ok());
    let trust = inputs
        .as_ref()
        .and_then(|inputs| policy::resolve_trust(inputs).ok())
        .unwrap_or_default();
    let trust_layer = policy::trust_ruleset(trust);
    let ruleset = match config {
        Some(config) => permissions::Ruleset::from_layers([base, config, trust_layer]),
        None => permissions::Ruleset::from_layers([base, trust_layer]),
    };

    // Seed the saved-decisions store from the shared per-project store on disk, keyed by the same
    // workspace key the checkpoint store uses, so the TUI and headless `cortex run` share one store.
    let loaded = permissions::store::load_for(&checkpoints::workspace_key(cwd));
    let saved = Arc::new(permissions::SavedDecisions::from_decisions(loaded.clone()));

    let responder: Arc<dyn permissions::Responder> = Arc::new(UiResponder::new(events));
    let engine = Arc::new(
        permissions::PermissionEngine::new(ruleset)
            .with_responder(responder)
            .with_saved_decisions(saved.clone()),
    );
    (engine, saved, loaded)
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

/// The TUI's default permission policy: allow everything, then ask before each effectful tool. With
/// last-matching-rule-wins the trailing `ask` rules govern the mutating/effectful tools while the
/// read-only tools stay allowed, so a normal read/grep/glob turn is never interrupted by a prompt.
fn default_ruleset() -> permissions::Ruleset {
    use permissions::{Action, Rule};
    permissions::Ruleset::new(vec![
        Rule::new("*", "*", Action::Allow),
        Rule::new("write", "*", Action::Ask),
        Rule::new("edit", "*", Action::Ask),
        Rule::new("apply_patch", "*", Action::Ask),
        Rule::new("bash", "*", Action::Ask),
    ])
}

/// Validate that a resolved model id is a fully-qualified `provider/model` OpenRouter id, rejecting
/// a bare id locally with an actionable message (mirroring the headless run's validation).
fn validate_model_id(model_id: &str) -> Result<(), String> {
    let (provider, model) = model_id.split_once('/').unwrap_or(("", model_id));
    if provider.is_empty() || model.is_empty() {
        return Err(format!(
            "unknown model '{model_id}': expected a fully-qualified 'provider/model' id \
             (for example google/gemini-2.5-flash)"
        ));
    }
    Ok(())
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

/// The OpenRouter endpoint override, if configured (the flat `openrouter_base_url` key, settable via
/// `CORTEX_OPENROUTER_BASE_URL`, wins over a nested `[provider.openrouter] base_url`).
fn openrouter_base_url_override(effective: &EffectiveConfig) -> Option<String> {
    top_level_string(effective, "openrouter_base_url")
        .or_else(|| provider_openrouter_string(effective, "base_url"))
}

/// The OpenRouter API key supplied through configuration (the lowest-precedence credential layer,
/// below the env var and the 0600 store).
fn openrouter_api_key(effective: &EffectiveConfig) -> Option<String> {
    provider_openrouter_string(effective, "api_key")
        .or_else(|| top_level_string(effective, "api_key"))
}

/// A non-empty top-level string config value (trimmed), or `None`.
fn top_level_string(effective: &EffectiveConfig, key: &str) -> Option<String> {
    effective
        .get(key)
        .and_then(|value| value.as_str())
        .and_then(non_empty)
}

/// A non-empty string value under the `[provider.openrouter]` table (trimmed), or `None`.
fn provider_openrouter_string(effective: &EffectiveConfig, key: &str) -> Option<String> {
    effective
        .get("provider")
        .and_then(|provider| provider.get("openrouter"))
        .and_then(|openrouter| openrouter.get(key))
        .and_then(|value| value.as_str())
        .and_then(non_empty)
}

/// Trim a candidate value and keep it only if non-empty after trimming.
fn non_empty(value: &str) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
