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
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::Duration;

use engine::{
    Agent, CompactionPolicy, EnvironmentBlock, RunnerEvent, SessionRunner, SystemContext, Turn,
};
use llm::Message;
use redaction::guard_outbound;
use settings::EffectiveConfig;

use crate::tooling::TurnDispatcher;

/// A message published by the turn worker for the UI thread to apply.
#[derive(Debug)]
pub enum TurnMessage {
    /// One normalized runner event (text/reasoning delta, usage, error, …).
    Event(RunnerEvent),
    /// A file changed by a tool this turn, rendered as a colored unified diff in the transcript.
    /// Boxed to keep the message enum small (a diff carries its full line list).
    FileDiff(Box<crate::diff::FileDiff>),
    /// The turn could not start: a setup-time diagnostic (net-disabled, no-credential, bad model).
    SetupError(String),
    /// The worker has finished; the UI clears the busy indicator and finalizes the turn.
    Finished,
}

/// The outbound operation named in a `net-disabled` refusal raised before the turn starts.
const OUTBOUND_OPERATION: &str = "LLM turn";

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
    cancel: Arc<AtomicBool>,
) -> Receiver<TurnMessage> {
    let (tx, rx) = mpsc::channel();
    let tx_events = tx.clone();
    thread::spawn(move || {
        match build_turn(&prompt, history) {
            Ok((provider, turn, model_id)) => {
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
                            let session_id = ephemeral_session_id();
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
                            let mut dispatcher =
                                TurnDispatcher::new(cwd, &model_id, session_id, tx_events.clone());
                            if let Some(recorder) = &recorder {
                                dispatcher = dispatcher.with_checkpoints(Arc::new(
                                    crate::checkpoint::RecorderBridge::new(recorder.clone()),
                                ));
                            }
                            let runner = SessionRunner::default();
                            // Race the streaming turn against the interrupt signal: if the UI asks
                            // to cancel, the runner future is dropped here (closing the upstream
                            // stream) and the turn stops streaming at a safe boundary.
                            tokio::select! {
                                _ = runner.run_with_tools(&provider, turn, &dispatcher, |event| {
                                    let _ = tx_events.send(TurnMessage::Event(event));
                                }) => {}
                                () = wait_for_cancel(cancel.clone()) => {}
                            }
                            // Reap any background `bash` jobs spawned this turn by exact PID, so no
                            // orphan process outlives the turn (even after an interrupt).
                            dispatcher.background_jobs().shutdown().await;
                            // Finalize the checkpoint AFTER the turn: record each touched file's
                            // post-mutation state and append the snapshot. A read-only turn records
                            // nothing, so it creates no checkpoint churn; a failure degrades undo for
                            // this turn rather than disrupting the rendered session.
                            if let Some(recorder) = &recorder {
                                let _ = recorder.finalize();
                            }
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

/// An ephemeral session id scoping this turn's tool context (the TUI does not yet persist sessions;
/// the id only keys the in-memory per-session todo list for the turn).
fn ephemeral_session_id() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    format!("tui-{}-{nanos}", std::process::id())
}

/// Build the provider and assembled [`Turn`] for a prompt, resolving the model, credential, and any
/// endpoint override through the same configuration precedence the headless run uses.
///
/// # Errors
/// Returns a human-readable, secret-scrubbed message when the kill-switch refuses the turn, a
/// configuration layer cannot be loaded, the model id is malformed, or no credential is available.
fn build_turn(
    prompt: &str,
    history: Vec<Message>,
) -> Result<(llm::OpenAiCompatible, Turn, String), String> {
    // Fail closed on the network kill-switch before resolving a credential or opening any socket.
    guard_outbound(OUTBOUND_OPERATION).map_err(|diag| diag.to_string())?;

    let inputs = settings::ConfigInputs::from_process(BTreeMap::new())
        .map_err(|err| format!("cannot determine the working directory: {err}"))?;
    let effective = settings::load(inputs).map_err(|err| err.to_string())?;

    let model = providers::resolve_model(Some(effective.model()));
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
    let system = SystemContext::for_model(&model_id, EnvironmentBlock::from_process())
        .with_agent(Agent::default())
        .render();
    let turn = Turn::new(&model_id, system, prompt)
        .with_history(history)
        .with_compaction(compaction);

    Ok((provider, turn, model_id))
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
}
