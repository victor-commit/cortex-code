//! The headless `cortex run` path: a single real, streamed turn against OpenRouter.
//!
//! A positional prompt argument is sent to the durable [`SessionRunner`](engine::SessionRunner)
//! and the assistant's answer is streamed to stdout as it arrives. The default output is clean
//! human prose (the streamed answer text only — no JSON envelope or event scaffolding); `--format
//! json` emits a single document built from the completed turn, and `--jsonl` streams the runner's
//! normalized event vocabulary one object per line.
//!
//! Input-source handling is deterministic: a positional prompt argument and piped (non-terminal)
//! stdin are combined when both are present (the piped content is treated as context that precedes
//! the instruction argument), so a large piped document reaches the model and exercises compaction.
//! With only one source present that source is the prompt; an interactive terminal is never read
//! (it would block). When no prompt is available at all (`</dev/null`, a closed pipe, or an
//! interactive terminal with nothing piped in) the command fails fast with [`Exit::MissingInput`]
//! rather than hanging.
//!
//! An over-large turn does not crash or hang: the runner's compaction policy (derived from the
//! resolved model's context window) prunes/truncates the request before sending and recovers from a
//! context-overflow provider error by compacting and rebuilding, otherwise surfacing a clear
//! context-overflow diagnostic (VAL-LLM-025/026).
//!
//! The model used for the turn is resolved through the full configuration precedence chain
//! (`defaults < global < project < nested < env < --model`): a config-file `model` is honored,
//! an explicit `--model` flag overrides it, and with neither present the mission default
//! `google/gemini-2.5-flash` is used. The model actually used is reported in machine output.
//!
//! The `CORTEX_NO_NET` kill-switch is consulted before a credential is resolved or any socket is
//! opened, so a run fails closed with a `net-disabled` diagnostic. When the credential is absent
//! the run fails with the canonical `no-credential` diagnostic; in neither case is any partial
//! answer written to stdout.

use std::collections::BTreeMap;
use std::io::{self, IsTerminal, Read, Write};
use std::sync::Arc;
use std::time::Duration;

use db::{Database, DbPathInputs, UsageTotals};
use engine::{
    Agent, CompactionPolicy, EnvironmentBlock, HistoryMessage, RunOutcome, RunnerEvent,
    SessionRunner, SystemContext, ToolDispatcher, Turn, project_history,
};
use errors::{Diagnostic, Exit};
use events::EventBus;
use protocol::ToolResultPart;
use providers::ResolvedModel;
use redaction::guard_outbound;
use serde::Serialize;
use settings::{ConfigError, ConfigInputs, EffectiveConfig};

use crate::cli::{OutputFormat, RunArgs};
use crate::permission;
use crate::permission_store;
use crate::tooling::RegistryDispatcher;

/// The outbound operation named in a `net-disabled` refusal raised before the turn starts.
const OUTBOUND_OPERATION: &str = "LLM turn";

/// Execute a single headless `cortex run` turn.
pub fn run(args: RunArgs, bus: Arc<EventBus>) -> Exit {
    let stdin = io::stdin();
    let stdin_is_terminal = stdin.is_terminal();
    let prompt = match resolve_prompt(args.prompt, stdin_is_terminal, stdin.lock()) {
        Ok(Some(prompt)) => prompt,
        Ok(None) => {
            eprintln!("cortex run: no input (pass a prompt argument or pipe text on stdin)");
            return Exit::MissingInput;
        }
        Err(err) => {
            eprintln!("cortex run: failed to read stdin: {err}");
            return Exit::Failure;
        }
    };

    let format = effective_format(args.jsonl, args.format);

    // Resolve the agent persona before any network consideration so an unknown agent fails fast
    // with an actionable, agent-naming error and a non-zero exit, never touching the network or the
    // durable store and never writing an assistant answer to stdout.
    let agent = match resolve_agent(args.agent.as_deref()) {
        Ok(agent) => agent,
        Err(err) => {
            eprintln!(
                "cortex run: {err}; select one with --agent <name> (for example --agent build)"
            );
            return Exit::Failure;
        }
    };

    // Parse the repeatable allow/deny CLI rule flags before any network or store work, so a
    // malformed rule fails fast with a usage error and zero side effects. These CLI rules are later
    // appended ON TOP of the config-file permission layers, so a CLI rule overrides a conflicting
    // config rule (last-matching-rule-wins).
    let cli_permission_rules =
        match permission::rules_from_flags(&args.permission_allow, &args.permission_deny) {
            Ok(rules) => rules,
            Err(message) => {
                eprintln!("cortex run: {message}");
                return Exit::Usage;
            }
        };

    // Parse the repeatable `--permission-allow-save` specs the same way: each is a `TOOL:GLOB`
    // "ask-and-remember (always allow)" pattern whose saved decision persists across runs. A
    // malformed spec fails fast with a usage error before any network or store work.
    let save_specs = match permission::rules_from_save_flags(&args.permission_allow_save) {
        Ok(rules) => rules,
        Err(message) => {
            eprintln!("cortex run: {message}");
            return Exit::Usage;
        }
    };

    // Fail closed on the network kill-switch before resolving a credential or opening any socket,
    // so a refusal never touches the network even when a key is present.
    if let Err(diag) = guard_outbound(OUTBOUND_OPERATION) {
        eprintln!("cortex run: {diag}");
        return diag.exit();
    }

    // Resolve the model, credential indirection, and any endpoint override through the full
    // configuration precedence chain so a config-file `model` is honored while an explicit
    // `--model` flag still wins. A config error (bad TOML, failed `{env:}`/`{file:}` substitution,
    // type mismatch) fails fast with the stable config exit code.
    let config_inputs = match ConfigInputs::from_process(model_overrides(args.model.as_deref())) {
        Ok(config_inputs) => config_inputs,
        Err(err) => {
            eprintln!("cortex run: cannot determine the working directory: {err}");
            return Exit::Failure;
        }
    };

    // The per-project working directory the saved-permission store is keyed by (identical to the
    // checkpoint store's key), captured before `config_inputs` is consumed by config resolution.
    let permission_cwd = config_inputs.cwd.clone();

    // Load this project's previously saved ("always allow/deny") decisions from the durable store
    // under the data dir, so a decision saved by an EARLIER, fully separate `cortex run` is honored
    // this run without re-asking (VAL-CROSS-027). A missing/corrupt store yields no decisions.
    let loaded_decisions = permission_store::load_for(&permission_cwd);

    // When an `--agent` is active, the permission BASE is that agent's effective ruleset (the
    // top-level `[[permission]]` base followed by the agent's `[[agent.<name>.permission]]`
    // overrides), so a per-agent rule gates the turn (VAL-PERM-009); with no `--agent` the base is
    // the top-level layered ruleset exactly as before. The flag's presence (not the resolved
    // default persona) decides this, so a default run keeps its unchanged top-level behavior.
    let active_agent = args.agent.is_some().then(|| agent.name());
    let builtin_agent_rules = permission::builtin_agent_permission_rules(agent);

    // Resolve the project's trust level (the layered config's `trust` key) into the highest-
    // precedence permission layer: an untrusted project denies the mutating/exec tools, so the SAME
    // action is blocked here that the TUI also blocks under untrusted, and a trusted project
    // contributes nothing (VAL-CROSS-030). A config error fails fast with the config exit code.
    let trust_rules = match policy::resolve_trust(&config_inputs) {
        Ok(level) => policy::trust_ruleset(level).rules().to_vec(),
        Err(err) => {
            eprintln!("cortex run: {err}");
            return Exit::Config;
        }
    };

    // Build the headless permission engine from every source `cortex run` honors: the config-file
    // `[[permission]]` layers (global → project → nested), agent-scoped when an `--agent` is active,
    // as the BASE ruleset, the CLI allow/deny rules appended ON TOP (last-matching-rule-wins), the
    // previously saved cross-process decisions (consulted before the ruleset, so a saved decision is
    // honored with no new ask), and the `--permission-allow-save` specs (each surfaced as an ask
    // answered "always allow" and recorded for reuse). The engine is attached whenever any of these
    // is non-empty; with none of them the run stays ungated (the default the other M2 tool features
    // rely on). A malformed config-file rule (or an unreadable/invalid config layer) fails fast with
    // the stable config exit code.
    let prepared_permissions = match permission::prepare_headless_permissions_with_builtin_rules(
        &config_inputs,
        active_agent,
        builtin_agent_rules.base_rules,
        cli_permission_rules,
        save_specs,
        builtin_agent_rules.restriction_rules,
        trust_rules,
        loaded_decisions,
        bus.clone(),
    ) {
        Ok(prepared) => prepared,
        Err(err) => {
            eprintln!("cortex run: {err}");
            return Exit::Config;
        }
    };
    let permission_engine = prepared_permissions.engine.clone();

    // Whether the caller passed an explicit `--model`: when present it always wins, so a model
    // persisted with the resumed session is NOT consulted; when absent a per-session model (if any)
    // governs the resume (VAL-CROSS-006).
    let explicit_model = args.model.is_some();

    let mut run_config = match resolve_run_config(config_inputs) {
        Ok(run_config) => run_config,
        Err(err) => {
            eprintln!("cortex run: {err}");
            return Exit::Config;
        }
    };

    // Reject a malformed model id locally, before any credential or socket, with an actionable
    // error naming the offending model. A well-formed but unknown id is left to the provider to
    // accept or reject upstream, preserving the OpenRouter passthrough for newer models.
    if let Err(message) = validate_model_id(&run_config.model.model_id) {
        eprintln!("cortex run: {message}");
        return Exit::Failure;
    }

    // The credential resolves through env > 0600 file > config indirection; an endpoint override
    // (config `provider.openrouter.base_url` / `openrouter_base_url`, the latter settable via
    // `CORTEX_OPENROUTER_BASE_URL`) retargets the transport for self-hosted gateways or tests.
    let inputs = providers::CredentialInputs::openrouter_from_process(run_config.api_key);
    let provider = match providers::resolve_openrouter_with_options(
        &inputs,
        run_config.base_url.as_deref(),
        stream_idle_timeout(),
    ) {
        Ok(provider) => provider,
        Err(diag) => return report_setup_diagnostic(&diag),
    };

    // Open the durable store and resolve the session this turn belongs to: an explicit `--session`
    // id (created on demand if unknown), the most recent session under `--continue` (a fresh one
    // when none exists), or a brand-new session otherwise. The store is touched only after setup
    // has succeeded, so a refused run (net-disabled, no-credential, bad model) leaves no session.
    let (db, db_path) = match open_database() {
        Ok(opened) => opened,
        Err(exit) => return exit,
    };
    let session_id = match resolve_session(&db, session_selection(args.session, args.r#continue)) {
        Ok(id) => id,
        Err(exit) => return exit,
    };

    // Honor a model persisted with the session (chosen interactively via the TUI `/models` picker)
    // when the caller passed no explicit `--model`: resuming the session headless then uses that
    // per-session model rather than reverting to the global default (VAL-CROSS-006). An explicit
    // `--model` always wins; a session with no stored model leaves the config-resolved model in
    // place. The stored id is validated the same way as a flag/config id, so a malformed stored
    // value fails fast rather than reaching the provider.
    let stored_model = db
        .sessions()
        .get(&session_id)
        .ok()
        .flatten()
        .and_then(|session| session.model);
    if let Some(model) = session_model_override(explicit_model, stored_model.as_deref()) {
        if let Err(message) = validate_model_id(&model) {
            eprintln!("cortex run: {message}");
            return Exit::Failure;
        }
        run_config.model = providers::resolve_model(Some(&model));
    }

    // Project the session's prior turns into canonical history BEFORE persisting this prompt, so
    // the new user input is sent once (as the turn's input) and not duplicated into the history.
    let history = match load_history(&db, &session_id) {
        Ok(history) => history,
        Err(exit) => return exit,
    };
    if let Err(err) = db.messages().create(&session_id, "user", &prompt) {
        eprintln!("cortex run: failed to persist the prompt: {err}");
        return Exit::Failure;
    }

    // The configured MCP servers for this run, taken before the model id is moved out of the config.
    let mcp_servers = std::mem::take(&mut run_config.mcp_servers);

    // Derive the compaction policy from the resolved model's context window before the model id is
    // moved out, so an over-large turn is compacted/recovered rather than crashing (VAL-LLM-025/026).
    let compaction = compaction_policy(&run_config.model);
    let model_id = run_config.model.model_id;
    let system = SystemContext::for_model(&model_id, EnvironmentBlock::from_process())
        .with_agent(agent)
        .render();
    let turn = Turn::new(&model_id, system, prompt)
        .with_history(history)
        .with_compaction(compaction);

    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(err) => {
            eprintln!("cortex run: failed to start the async runtime: {err}");
            return Exit::Failure;
        }
    };

    // Connect every configured MCP server (spawn the stdio child, handshake, list tools) before the
    // turn so its server-qualified tools materialize into the per-turn catalogue. A failed/disabled
    // server is recorded and skipped — it never blocks the turn. Connection happens on the runtime
    // because it is async. With no configured servers this is a clean no-op (no children spawned).
    let mcp_manager: Option<std::sync::Arc<mcp::McpManager>> = if mcp_servers.is_empty() {
        None
    } else {
        let options = mcp::McpConnectOptions {
            bus: Some(bus.clone()),
            credentials: crate::mcp_store::store_from_process(),
        };
        Some(std::sync::Arc::new(runtime.block_on(
            mcp::McpManager::connect_with(mcp_servers, options),
        )))
    };

    // Materialize the per-turn tool set for the model's family, scoped to the process working
    // directory, so the model can read/edit the filesystem for real this turn. A GPT-family model
    // gets `apply_patch` (and loses `edit`/`write`); any other model keeps `edit`/`write`
    // (architecture §3.4 / §4).
    let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    let todo_store: std::sync::Arc<dyn tools::TodoStore> =
        std::sync::Arc::new(crate::todo_store::DbTodoStore::new(db_path));

    // Open the shadow checkpoint store for the working directory and begin recording this turn, so a
    // pre-mutation snapshot is captured BEFORE any write/edit/apply_patch changes the tree (powering
    // `cortex undo`/`cortex redo`). Checkpointing degrades silently when the store cannot be opened
    // (git unavailable or no resolvable data location) — it never blocks the run.
    let checkpoint_recorder = match crate::checkpoint::open_store(&cwd) {
        Ok(Some(store)) => Some(std::sync::Arc::new(store.begin_turn(session_id.clone()))),
        Ok(None) => None,
        Err(err) => {
            eprintln!("cortex run: checkpoint store unavailable: {err}");
            None
        }
    };

    let lsp_hooks = std::sync::Arc::new(crate::lsp_bridge::LspHooks::new(cwd.clone()));
    let mutation_hooks = std::sync::Arc::new(crate::format_bridge::PostMutationHooks::with_config(
        cwd.clone(),
        run_config.formatter_config.clone(),
        lsp_hooks.clone(),
    ));

    let dispatcher = RegistryDispatcher::for_model(cwd, &model_id, session_id.clone(), todo_store);
    // Attach the permission engine (when rules were supplied) so the headless tool context is gated
    // by the same engine the tools already call via `ctx.permits`.
    let dispatcher = match permission_engine {
        Some(engine) => dispatcher.with_permissions(engine),
        None => dispatcher,
    };
    // Attach the checkpoint recorder so each mutating tool snapshots a file's prior state before it
    // changes it.
    let dispatcher = match &checkpoint_recorder {
        Some(recorder) => dispatcher.with_checkpoints(std::sync::Arc::new(
            crate::checkpoint::RecorderBridge::new(recorder.clone()),
        )),
        None => dispatcher,
    };
    // Attach the real formatter + LSP diagnostics hook so write/edit/apply_patch first canonicalize
    // the on-disk file and then report diagnostics from the matching language server.
    let dispatcher = dispatcher.with_hooks(mutation_hooks);
    // Attach the same real LSP manager to the read-only `lsp` navigation tool, so hover,
    // definition, references, and symbols use the live server and share its lifecycle.
    let dispatcher = dispatcher.with_lsp_service(lsp_hooks.clone());
    // Attach the connected MCP manager so its server-qualified tools are advertised and callable
    // alongside the built-in tools this turn.
    let dispatcher = match &mcp_manager {
        Some(manager) => dispatcher.with_mcp(manager.clone()),
        None => dispatcher,
    };
    let tool_names = dispatcher.tool_names();

    let exit = runtime.block_on(async {
        let exit = match format {
            OutputFormat::Text => stream_text(&provider, &session_id, &db, turn, &dispatcher).await,
            OutputFormat::Jsonl => {
                stream_jsonl(
                    &provider,
                    agent.name(),
                    &session_id,
                    &tool_names,
                    &db,
                    turn,
                    &dispatcher,
                )
                .await
            }
            OutputFormat::Json => {
                emit_json(
                    &provider,
                    &model_id,
                    agent.name(),
                    &session_id,
                    &tool_names,
                    &db,
                    turn,
                    &dispatcher,
                )
                .await
            }
        };
        // Reap any background `bash` jobs spawned this turn by exact PID, so no orphan process
        // outlives the run.
        dispatcher.background_jobs().shutdown().await;
        // Terminate every spawned LSP server by its child PID before the process exits.
        lsp_hooks.shutdown();
        // Terminate every spawned MCP server (cancel its session, killing the stdio child by PID),
        // so no MCP server process outlives the run.
        if let Some(manager) = &mcp_manager {
            manager.shutdown().await;
        }
        exit
    });

    // Finalize the turn's checkpoint AFTER the turn completes: record each touched file's
    // post-mutation state and append the snapshot to the durable journal. A read-only turn (no
    // mutations recorded) appends nothing, so purely read/grep/glob turns create no checkpoint churn.
    if let Some(recorder) = &checkpoint_recorder {
        crate::checkpoint::finalize_turn(recorder);
    }

    // Persist any "always allow/deny" decisions saved this run to the per-project store, so a LATER,
    // fully separate `cortex run` on this project reuses them without re-asking (VAL-CROSS-027). Only
    // a run that could record a decision (`--permission-allow-save`) rewrites the store; a persist
    // failure is a stderr warning, not a non-zero exit (the answer already shipped).
    if prepared_permissions.persist {
        if let Err(err) =
            permission_store::save_for(&permission_cwd, &prepared_permissions.saved.snapshot())
        {
            eprintln!("cortex run: failed to persist saved permission decisions: {err}");
        }
    }
    exit
}

/// Resolve the agent persona for the run from the optional `--agent` flag, defaulting to the
/// built-in default agent when the flag is absent.
///
/// # Errors
/// Returns [`engine::UnknownAgent`] (naming the offending input and the available agents) when the
/// supplied name matches no built-in persona.
fn resolve_agent(name: Option<&str>) -> Result<Agent, engine::UnknownAgent> {
    match name {
        Some(name) => Agent::from_name(name),
        None => Ok(Agent::default()),
    }
}

/// How this run selects the session it belongs to.
enum SessionSelection {
    /// Start a fresh session with a store-generated id (no `--session`/`--continue`).
    New,
    /// Resume (or create on first use) the caller-supplied session id (`--session <id>`).
    Explicit(String),
    /// Attach to the most recent session, or start a fresh one when none exists (`--continue`).
    Continue,
}

/// Map the mutually-exclusive `--session`/`--continue` flags to a [`SessionSelection`]. An explicit
/// id wins; `--continue` attaches to the latest session; with neither a new session is created.
fn session_selection(session: Option<String>, continue_latest: bool) -> SessionSelection {
    match session {
        Some(id) => SessionSelection::Explicit(id),
        None if continue_latest => SessionSelection::Continue,
        None => SessionSelection::New,
    }
}

/// Open the durable session store at the resolved database path, mapping any failure to a clear
/// stderr diagnostic and a non-zero exit (never a panic). The resolved path is returned alongside
/// the open database so the per-session `todo` store can open its own connection to the same file.
fn open_database() -> Result<(Database, std::path::PathBuf), Exit> {
    let Some(path) = DbPathInputs::from_process().resolve() else {
        eprintln!(
            "cortex run: cannot determine a database location; set HOME, XDG_DATA_HOME, \
             CORTEX_DATA_DIR, or CORTEX_DB_PATH"
        );
        return Err(Exit::Failure);
    };
    Database::open(&path).map(|db| (db, path)).map_err(|err| {
        eprintln!("cortex run: {err}");
        Exit::Failure
    })
}

/// Resolve the session id this run uses, creating or resuming durable state as the selection
/// dictates. `--continue` on a store with no prior session deterministically starts a fresh one,
/// so it terminates cleanly rather than erroring (VAL-RUN-010).
fn resolve_session(db: &Database, selection: SessionSelection) -> Result<String, Exit> {
    let resolved = match selection {
        SessionSelection::New => db.sessions().create(None).map(|session| session.id),
        SessionSelection::Explicit(id) => db.sessions().ensure(&id, None).map(|session| session.id),
        SessionSelection::Continue => match db.sessions().most_recent() {
            Ok(Some(session)) => Ok(session.id),
            Ok(None) => db.sessions().create(None).map(|session| session.id),
            Err(err) => Err(err),
        },
    };
    resolved.map_err(|err| {
        eprintln!("cortex run: {err}");
        Exit::Failure
    })
}

/// Load a session's stored turns and project them into canonical [`llm::Message`] history.
fn load_history(db: &Database, session_id: &str) -> Result<Vec<llm::Message>, Exit> {
    match db.messages().list_for_session(session_id) {
        Ok(messages) => {
            let history: Vec<HistoryMessage> = messages
                .into_iter()
                .map(|message| HistoryMessage::new(message.role, message.content))
                .collect();
            Ok(project_history(&history))
        }
        Err(err) => {
            eprintln!("cortex run: failed to load session history: {err}");
            Err(Exit::Failure)
        }
    }
}

/// Persist the assistant's reply so a later run resuming the same session sees it. A persistence
/// failure is reported on stderr but does not change the exit code: the answer has already been
/// delivered to the user, so the run itself succeeded.
fn persist_assistant(db: &Database, session_id: &str, outcome: &RunOutcome) {
    if let Err(err) = db
        .messages()
        .create(session_id, "assistant", outcome.answer())
    {
        eprintln!("cortex run: failed to persist the assistant reply: {err}");
    }
}

/// Record this turn's token usage and cost so the session's cumulative totals stay accurate across
/// separate `cortex run` invocations. Recorded in every output format (not just JSON) so a session
/// that mixes formats still accumulates correctly. A persistence failure is reported on stderr but
/// does not change the exit code: the answer has already been delivered, so the run itself
/// succeeded.
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
        eprintln!("cortex run: failed to persist usage: {err}");
    }
}

/// The session's cumulative usage/cost across every recorded turn (this turn included), or `None`
/// when the totals cannot be read. Reported alongside the per-turn usage so a machine consumer sees
/// a running, monotonically non-decreasing session total rather than a single turn's value.
fn session_usage_totals(db: &Database, session_id: &str) -> Option<UsageTotals> {
    match db.usage().session_totals(session_id) {
        Ok(totals) => Some(totals),
        Err(err) => {
            eprintln!("cortex run: failed to read session usage totals: {err}");
            None
        }
    }
}

/// Report a setup-time provider diagnostic to stderr (keeping stdout byte-empty) and return its
/// mapped exit code. A missing-credential diagnostic is followed by an actionable hint naming how
/// to supply the key, so a keyless run tells the user exactly what to set (VAL-LLM-022).
fn report_setup_diagnostic(diag: &Diagnostic) -> Exit {
    eprintln!("cortex run: {diag}");
    if let Diagnostic::NoCredential(provider) = diag {
        if provider == providers::OPENROUTER_ID {
            eprintln!(
                "cortex run: set the {} environment variable (or store a key in the credential \
                 file or config) to authenticate with OpenRouter",
                providers::OPENROUTER_ENV
            );
        } else {
            eprintln!(
                "cortex run: provide an API key for {provider} via its environment variable, the \
                 credential file, or config"
            );
        }
    }
    diag.exit()
}

/// The result of awaiting a turn that may be interrupted by SIGINT.
enum TurnResult {
    /// The turn ran to completion (success or a normalized error in the outcome).
    Completed(RunOutcome),
    /// SIGINT (Ctrl-C) cancelled the turn at a safe await boundary before it completed.
    Interrupted,
}

/// Run a turn against the provider, racing it against a SIGINT so the process cancels cleanly at a
/// safe boundary instead of being hard-killed mid-write.
///
/// On interrupt the in-flight runner future is dropped, which drops the upstream SSE stream and
/// aborts the connection (no orphaned children, no panic), and [`TurnResult::Interrupted`] is
/// returned so the caller can finalize output cleanly (VAL-RUN-005 / VAL-RUN-013).
async fn run_turn<F>(
    provider: &dyn llm::Provider,
    turn: Turn,
    dispatcher: &dyn ToolDispatcher,
    sink: F,
) -> TurnResult
where
    F: FnMut(RunnerEvent),
{
    let runner = SessionRunner::default();
    tokio::select! {
        outcome = runner.run_with_tools(provider, turn, dispatcher, sink) => {
            TurnResult::Completed(outcome)
        }
        () = interrupt() => TurnResult::Interrupted,
    }
}

/// Resolve when the process receives a SIGINT (Ctrl-C).
///
/// If the signal handler cannot be installed the future never resolves, so the turn proceeds and
/// the OS default disposition applies rather than silently swallowing interrupts.
async fn interrupt() {
    if tokio::signal::ctrl_c().await.is_err() {
        std::future::pending::<()>().await;
    }
}

/// The per-frame idle/stall timeout for the streamed turn, read from the operational
/// `CORTEX_STREAM_IDLE_TIMEOUT_MS` environment variable.
///
/// When set to a positive integer the transport aborts a stalled upstream within that bounded
/// window with a transport diagnostic (fail-closed) instead of hanging (VAL-RUN-004); an absent,
/// blank, zero, or unparseable value keeps the transport default.
fn stream_idle_timeout() -> Option<Duration> {
    parse_idle_timeout_ms(
        std::env::var("CORTEX_STREAM_IDLE_TIMEOUT_MS")
            .ok()
            .as_deref(),
    )
}

/// Parse a millisecond idle-timeout value, accepting only a positive integer (trimmed). Returns
/// `None` for an absent, blank, zero, or unparseable value so the transport default stands.
fn parse_idle_timeout_ms(raw: Option<&str>) -> Option<Duration> {
    let ms: u64 = raw?.trim().parse().ok()?;
    (ms >= 1).then(|| Duration::from_millis(ms))
}

/// Stream the assistant answer as clean prose, flushing each fragment as it arrives so output is
/// incremental rather than a single end-of-turn dump. The session id is intentionally omitted from
/// the human prose (it is available via the machine formats).
async fn stream_text(
    provider: &dyn llm::Provider,
    session_id: &str,
    db: &Database,
    turn: Turn,
    dispatcher: &dyn ToolDispatcher,
) -> Exit {
    let stdout = io::stdout();
    let mut handle = stdout.lock();
    let mut wrote_any = false;

    let result = run_turn(provider, turn, dispatcher, |event| {
        if let RunnerEvent::TextDelta { text } = event {
            if !text.is_empty() {
                let _ = handle.write_all(text.as_bytes());
                let _ = handle.flush();
                wrote_any = true;
            }
        }
    })
    .await;

    let outcome = match result {
        TurnResult::Completed(outcome) => outcome,
        TurnResult::Interrupted => {
            // Cancelled at a safe boundary: finish the partial line cleanly and exit with the
            // interrupt code. The session stays durable (the prompt was persisted before the turn).
            if wrote_any {
                let _ = handle.write_all(b"\n");
                let _ = handle.flush();
            }
            drop(handle);
            eprintln!("cortex run: interrupted");
            return Exit::Interrupted;
        }
    };

    if let Some(err) = &outcome.error {
        if wrote_any {
            let _ = handle.write_all(b"\n");
            let _ = handle.flush();
        }
        drop(handle);
        eprintln!("cortex run: {err}");
        return err.exit();
    }

    // Terminate the streamed answer with a newline so shell capture and the next prompt are clean,
    // without disturbing the model's own trailing newline if it already emitted one.
    if wrote_any && !outcome.answer().ends_with('\n') {
        let _ = handle.write_all(b"\n");
        let _ = handle.flush();
    }
    drop(handle);
    persist_assistant(db, session_id, &outcome);
    persist_usage(db, session_id, &outcome);
    Exit::Success
}

/// Stream the runner's normalized event vocabulary, one JSON object per line. A leading `session`
/// event announces the session id so a machine consumer can capture and reuse it via `--session`.
#[allow(clippy::too_many_arguments)]
async fn stream_jsonl(
    provider: &dyn llm::Provider,
    agent: &str,
    session_id: &str,
    tool_names: &[String],
    db: &Database,
    turn: Turn,
    dispatcher: &dyn ToolDispatcher,
) -> Exit {
    let stdout = io::stdout();
    let mut handle = stdout.lock();

    if let Ok(line) = render_session_line(agent, session_id, tool_names) {
        let _ = handle.write_all(line.as_bytes());
        let _ = handle.write_all(b"\n");
        let _ = handle.flush();
    }

    let result = run_turn(provider, turn, dispatcher, |event| {
        if let Ok(line) = render_event_line(&event) {
            let _ = handle.write_all(line.as_bytes());
            let _ = handle.write_all(b"\n");
            let _ = handle.flush();
        }
    })
    .await;

    let outcome = match result {
        TurnResult::Completed(outcome) => outcome,
        TurnResult::Interrupted => {
            // Every JSONL line already written is a complete object; stop emitting and exit. The
            // session stays durable (the prompt was persisted before the turn).
            drop(handle);
            eprintln!("cortex run: interrupted");
            return Exit::Interrupted;
        }
    };

    drop(handle);
    if let Some(err) = &outcome.error {
        eprintln!("cortex run: {err}");
        return err.exit();
    }
    persist_assistant(db, session_id, &outcome);
    persist_usage(db, session_id, &outcome);
    Exit::Success
}

/// A single JSON document describing a completed turn for `--format json`.
#[derive(Serialize)]
struct RunDocument<'a> {
    role: &'a str,
    content: &'a str,
    model: &'a str,
    /// The agent persona this turn ran under (the value passed to `--agent`, or the default).
    agent: &'a str,
    /// The stable, re-usable session id this turn ran under (feed back via `--session`).
    session: &'a str,
    finish_reason: &'a llm::FinishReason,
    /// How many provider round-trips (steps) the turn took; always within the `MAX_STEPS` budget
    /// so a machine consumer observes a bounded iteration count (VAL-RUN-003).
    steps: usize,
    /// The per-turn token usage and cost for this turn.
    #[serde(skip_serializing_if = "Option::is_none")]
    usage: Option<&'a llm::Usage>,
    /// The session's cumulative token usage and cost across every turn so far (this turn included),
    /// so a consumer sees a running, monotonically non-decreasing session total.
    #[serde(skip_serializing_if = "Option::is_none")]
    session_usage: Option<UsageTotals>,
    /// The per-turn tool set materialized for the model's family, in advertised order. A GPT-family
    /// model lists `apply_patch` (and no `edit`/`write`); any other model lists `edit`/`write`
    /// (and no `apply_patch`). This is the observable per-turn tool advertisement (VAL-TOOLS-019/020).
    tools: &'a [String],
    /// The structured results of every tool the model invoked this turn, in call order. Carries
    /// text parts and any media attachments (images/PDFs) so a machine consumer sees the tool
    /// output without binary bytes leaking into the text channel. Omitted when no tool was called.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tool_results: Vec<ToolResultDoc>,
}

/// One tool result in the completed-turn document: the call id, the tool name, the structured
/// result parts, and whether the tool reported an error.
#[derive(Serialize)]
struct ToolResultDoc {
    id: String,
    name: String,
    parts: Vec<ToolResultPart>,
    is_error: bool,
}

/// The leading `--jsonl` event announcing the agent persona and session id, before the runner's
/// own events.
#[derive(Serialize)]
struct SessionEvent<'a> {
    #[serde(rename = "type")]
    kind: &'a str,
    /// The agent persona this turn ran under (the value passed to `--agent`, or the default).
    agent: &'a str,
    /// The stable, re-usable session id this turn ran under (feed back via `--session`).
    session: &'a str,
    /// The per-turn tool set materialized for the model's family, in advertised order (the
    /// observable per-turn tool advertisement; see [`RunDocument::tools`]).
    tools: &'a [String],
}

/// Run the turn to completion, then emit a single strict JSON document on stdout. No partial
/// document is written if the turn fails.
#[allow(clippy::too_many_arguments)]
async fn emit_json(
    provider: &dyn llm::Provider,
    model_id: &str,
    agent: &str,
    session_id: &str,
    tool_names: &[String],
    db: &Database,
    turn: Turn,
    dispatcher: &dyn ToolDispatcher,
) -> Exit {
    // Collect each tool result as it streams so the completed document can carry the structured
    // tool output (text plus any media attachments) alongside the assistant answer.
    let mut tool_results: Vec<ToolResultDoc> = Vec::new();
    let result = run_turn(provider, turn, dispatcher, |event| {
        if let RunnerEvent::ToolResult {
            id,
            name,
            parts,
            is_error,
        } = event
        {
            tool_results.push(ToolResultDoc {
                id,
                name,
                parts,
                is_error,
            });
        }
    })
    .await;

    let outcome = match result {
        TurnResult::Completed(outcome) => outcome,
        TurnResult::Interrupted => {
            // The single JSON document is written only after the turn completes, so an interrupt
            // here leaves stdout byte-empty — never a partial/malformed document (VAL-RUN-013).
            eprintln!("cortex run: interrupted");
            return Exit::Interrupted;
        }
    };

    if let Some(err) = &outcome.error {
        eprintln!("cortex run: {err}");
        return err.exit();
    }

    // Persist this turn's usage BEFORE reading the session totals so the reported cumulative figure
    // includes the current turn; tie the session total to a turn that actually reported usage.
    persist_usage(db, session_id, &outcome);
    let session_usage = if outcome.usage.is_some() {
        session_usage_totals(db, session_id)
    } else {
        None
    };

    let line = match render_json_document(
        model_id,
        agent,
        session_id,
        tool_names,
        &outcome,
        session_usage,
        tool_results,
    ) {
        Ok(line) => line,
        Err(err) => {
            eprintln!("cortex run: failed to serialize output: {err}");
            return Exit::Failure;
        }
    };

    let stdout = io::stdout();
    let mut handle = stdout.lock();
    if handle
        .write_all(line.as_bytes())
        .and_then(|()| handle.flush())
        .is_err()
    {
        return Exit::Failure;
    }
    drop(handle);
    persist_assistant(db, session_id, &outcome);
    Exit::Success
}

/// Build the single JSON document describing a completed turn (role, content, model, session id,
/// finish reason, and the optional usage object).
fn build_run_document<'a>(
    model_id: &'a str,
    agent: &'a str,
    session_id: &'a str,
    tool_names: &'a [String],
    outcome: &'a RunOutcome,
    session_usage: Option<UsageTotals>,
    tool_results: Vec<ToolResultDoc>,
) -> RunDocument<'a> {
    RunDocument {
        role: "assistant",
        content: outcome.answer(),
        model: model_id,
        agent,
        session: session_id,
        finish_reason: &outcome.finish_reason,
        steps: outcome.steps,
        usage: outcome.usage.as_ref(),
        session_usage,
        tools: tool_names,
        tool_results,
    }
}

/// Serialize the completed-turn document to a single line terminated by exactly one newline, so
/// stdout carries one strict JSON value with no leading or trailing non-JSON bytes.
#[allow(clippy::too_many_arguments)]
fn render_json_document(
    model_id: &str,
    agent: &str,
    session_id: &str,
    tool_names: &[String],
    outcome: &RunOutcome,
    session_usage: Option<UsageTotals>,
    tool_results: Vec<ToolResultDoc>,
) -> Result<String, serde_json::Error> {
    let mut line = serde_json::to_string(&build_run_document(
        model_id,
        agent,
        session_id,
        tool_names,
        outcome,
        session_usage,
        tool_results,
    ))?;
    line.push('\n');
    Ok(line)
}

/// Render the leading `--jsonl` session event (without the trailing newline). It announces the
/// agent persona, the session id, and the per-turn tool advertisement.
fn render_session_line(
    agent: &str,
    session_id: &str,
    tool_names: &[String],
) -> Result<String, serde_json::Error> {
    serde_json::to_string(&SessionEvent {
        kind: "session",
        agent,
        session: session_id,
        tools: tool_names,
    })
}

/// Render one normalized runner event as a single JSONL line (without the trailing newline). Every
/// event serializes to an object carrying a stable `type` discriminator.
fn render_event_line(event: &RunnerEvent) -> Result<String, serde_json::Error> {
    serde_json::to_string(event)
}

/// Collapse the two output-selection inputs into a single effective format. The `--jsonl`
/// shorthand is mutually exclusive with `--format` at the arg layer, so it wins only when set.
fn effective_format(jsonl: bool, format: OutputFormat) -> OutputFormat {
    if jsonl { OutputFormat::Jsonl } else { format }
}

/// Build the CLI-layer configuration overrides for the run from the optional `--model` flag.
///
/// An explicit `--model` becomes the highest-precedence `model` override (beating env and every
/// config file); its absence yields an empty override map so the lower layers (config files, env,
/// built-in default) decide the model.
fn model_overrides(cli_model: Option<&str>) -> BTreeMap<String, String> {
    let mut overrides = BTreeMap::new();
    if let Some(model) = cli_model {
        overrides.insert("model".to_owned(), model.to_owned());
    }
    overrides
}

/// The configuration-derived inputs for a run: the resolved model plus the optional OpenRouter
/// endpoint override and config-supplied credential (the lowest-precedence credential layer).
struct RunConfig {
    model: ResolvedModel,
    base_url: Option<String>,
    api_key: Option<String>,
    mcp_servers: std::collections::BTreeMap<String, mcp::McpServerConfig>,
    formatter_config: format::FormatterConfig,
}

/// Resolve everything a run derives from configuration: merge every layer (the `--model` flag is
/// carried as the CLI override inside `inputs`), resolve the winning model id against the provider
/// catalogue, and extract the optional OpenRouter endpoint override and config-supplied API key.
///
/// # Errors
/// Returns the underlying [`ConfigError`] when a configuration layer cannot be loaded (bad TOML,
/// failed `{env:}`/`{file:}` substitution, or a type mismatch).
fn resolve_run_config(inputs: ConfigInputs) -> Result<RunConfig, ConfigError> {
    let effective = settings::load(inputs)?;
    let formatter_config =
        format::FormatterConfig::from_table(effective.table()).map_err(|message| {
            ConfigError::TypeMismatch {
                key: "formatter".to_owned(),
                expected: "a formatter config table",
                found: message,
            }
        })?;
    Ok(RunConfig {
        model: providers::resolve_model(Some(effective.model())),
        base_url: openrouter_base_url_override(&effective),
        api_key: openrouter_api_key(&effective),
        mcp_servers: mcp_servers_from_config(&effective),
        formatter_config,
    })
}

/// Extract the configured MCP servers from the effective config's `mcp` table. A malformed `mcp`
/// entry is surfaced as a stderr warning and skipped rather than failing the whole run, so a broken
/// server definition never blocks an otherwise-valid turn.
pub(crate) fn mcp_servers_from_config(
    effective: &EffectiveConfig,
) -> std::collections::BTreeMap<String, mcp::McpServerConfig> {
    let Some(value) = effective.get("mcp") else {
        return std::collections::BTreeMap::new();
    };
    let json = serde_json::to_value(value).unwrap_or(serde_json::Value::Null);
    match mcp::servers_from_value(&json) {
        Ok(servers) => servers,
        Err(err) => {
            eprintln!("cortex run: ignoring invalid MCP configuration: {err}");
            std::collections::BTreeMap::new()
        }
    }
}

/// Resolve the effective model for the turn (a thin wrapper over [`resolve_run_config`]).
///
/// # Errors
/// Returns the underlying [`ConfigError`] when a configuration layer cannot be loaded.
#[cfg(test)]
fn resolve_run_model(inputs: ConfigInputs) -> Result<ResolvedModel, ConfigError> {
    Ok(resolve_run_config(inputs)?.model)
}

/// The OpenRouter endpoint override, if configured. The flat `openrouter_base_url` key (settable
/// via the `CORTEX_OPENROUTER_BASE_URL` environment variable through the standard `CORTEX_<KEY>`
/// mapping) wins over a nested `[provider.openrouter] base_url`, so an env/CLI override is
/// authoritative. An empty/whitespace value is treated as absent.
fn openrouter_base_url_override(effective: &EffectiveConfig) -> Option<String> {
    top_level_string(effective, "openrouter_base_url")
        .or_else(|| provider_openrouter_string(effective, "base_url"))
}

/// The OpenRouter API key supplied through configuration (the lowest-precedence credential layer,
/// below the env var and the 0600 store). A nested `[provider.openrouter] api_key` wins over a
/// flat `api_key`. Any `{env:}`/`{file:}` indirection has already been resolved by `settings`.
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

/// Build the compaction policy for the run from the resolved model's context window and per-turn
/// output limit. The effective window can be overridden via `CORTEX_CONTEXT_LIMIT_TOKENS` (an
/// operational tuning/testing knob); an unknown model falls back to the engine's default window.
fn compaction_policy(model: &ResolvedModel) -> CompactionPolicy {
    if let Some(context) = context_limit_override() {
        // An explicit override drives the budget purely from the requested window.
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
/// `CORTEX_CONTEXT_LIMIT_TOKENS`.
fn context_limit_override() -> Option<usize> {
    parse_context_limit(std::env::var("CORTEX_CONTEXT_LIMIT_TOKENS").ok().as_deref())
}

/// Parse a context-window override token, accepting only a positive integer (trimmed). Returns
/// `None` for an absent, blank, zero, or unparseable value so the model-derived window stands.
fn parse_context_limit(raw: Option<&str>) -> Option<usize> {
    let parsed: usize = raw?.trim().parse().ok()?;
    (parsed >= 1).then_some(parsed)
}

/// Validate that a resolved model id is a fully-qualified `provider/model` OpenRouter id.
///
/// OpenRouter model ids always carry a provider prefix (e.g. `google/gemini-2.5-flash`), so a
/// bare id without one is not a valid model. Rejecting it here fails fast with an actionable error
/// naming the offending model rather than letting the provider silently auto-route an ambiguous
/// request. A well-formed but unknown id is intentionally accepted (it is left to the provider to
/// accept or reject upstream), which preserves the OpenRouter passthrough for models newer than
/// the embedded catalogue.
///
/// # Errors
/// Returns an actionable, model-naming message when the id is not a `provider/model` pair.
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

/// Decide whether a session's persisted model should override the config-resolved model for a
/// resumed run.
///
/// A persisted, non-empty model is honored only when the caller passed no explicit `--model`
/// (which always wins); a blank or absent stored model leaves the config-resolved model in place.
/// Returned trimmed and owned so the caller can validate and resolve it (VAL-CROSS-006).
fn session_model_override(explicit_model: bool, session_model: Option<&str>) -> Option<String> {
    if explicit_model {
        return None;
    }
    session_model
        .map(str::trim)
        .filter(|model| !model.is_empty())
        .map(str::to_owned)
}

/// Resolve the prompt according to the documented precedence (argument over stdin).
///
/// `stdin_is_terminal` is passed in (rather than derived from `source`) because `IsTerminal`
/// is a sealed trait that cannot be implemented for test doubles. Returns `Ok(None)` when no
/// prompt is available.
fn resolve_prompt<R: Read>(
    arg: Option<String>,
    stdin_is_terminal: bool,
    mut source: R,
) -> io::Result<Option<String>> {
    // Read piped (non-terminal) stdin once. An interactive terminal is never read because it would
    // block forever waiting on a human; a closed/empty pipe normalizes to "no input".
    let piped = if stdin_is_terminal {
        None
    } else {
        let mut buffer = String::new();
        source.read_to_string(&mut buffer)?;
        normalize_stdin(&buffer)
    };

    Ok(match (arg, piped) {
        // Both present: the piped content is context (e.g. a large document or "padding") that the
        // instruction argument refers to, so it is prepended to the instruction as one prompt. This
        // is what lets a large piped input reach the model and exercise compaction/overflow
        // recovery (VAL-LLM-025) while keeping the short instruction intact.
        (Some(arg), Some(piped)) => Some(combine_prompt(&piped, &arg)),
        (Some(arg), None) => Some(arg),
        (None, Some(piped)) => Some(piped),
        (None, None) => None,
    })
}

/// Combine piped stdin context with the positional instruction: the stdin content first (the
/// material the instruction refers to), a blank line, then the instruction.
fn combine_prompt(piped: &str, arg: &str) -> String {
    format!("{piped}\n\n{arg}")
}

/// Normalize raw stdin into an optional prompt: strip a single trailing line ending (the one
/// a shell pipe/`echo` typically appends) and treat the empty result as "no input".
fn normalize_stdin(raw: &str) -> Option<String> {
    let trimmed = strip_one_trailing_newline(raw);
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_owned())
    }
}

/// Strip at most one trailing line ending (`\n` or `\r\n`), preserving any earlier newlines.
fn strip_one_trailing_newline(s: &str) -> &str {
    match s.strip_suffix('\n') {
        Some(without_lf) => without_lf.strip_suffix('\r').unwrap_or(without_lf),
        None => s,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU32, Ordering};

    /// A non-terminal stdin double (a pipe/redirect), so `resolve_prompt` reads it.
    fn piped(data: &str) -> Cursor<Vec<u8>> {
        Cursor::new(data.as_bytes().to_vec())
    }

    /// A unique, freshly created empty temp directory for hermetic config-layer tests.
    fn scratch_dir(tag: &str) -> PathBuf {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let unique = format!(
            "cortex-model-sel-{tag}-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        );
        let dir = std::env::temp_dir().join(unique);
        std::fs::create_dir_all(&dir).expect("create scratch dir");
        dir
    }

    /// Build a fresh `base` (home) + `base/proj` (cwd) scratch tree so the project-config walk is
    /// bounded to the scratch dir (home is the walk boundary) and never reads real config files.
    fn scratch_tree(tag: &str) -> (PathBuf, PathBuf) {
        let base = scratch_dir(tag);
        let proj = base.join("proj");
        std::fs::create_dir_all(&proj).expect("create proj dir");
        (base, proj)
    }

    /// Construct hermetic [`ConfigInputs`]: `home` bounds the project walk, `cwd` is where project
    /// config is discovered, plus explicit `CORTEX_*` env and the optional `--model` CLI override.
    fn config_inputs(
        home: &Path,
        cwd: &Path,
        env: &[(&str, &str)],
        cli_model: Option<&str>,
    ) -> ConfigInputs {
        ConfigInputs {
            home: Some(home.to_path_buf()),
            xdg_config_home: None,
            cwd: cwd.to_path_buf(),
            env: env
                .iter()
                .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
                .collect(),
            cli_overrides: model_overrides(cli_model),
        }
    }

    /// Write a project `cortex.toml` selecting `model` into the given directory.
    fn write_config_model(dir: &Path, model: &str) {
        std::fs::write(dir.join("cortex.toml"), format!("model = \"{model}\"\n"))
            .expect("write cortex.toml");
    }

    /// Write an arbitrary project `cortex.toml` body into the given directory.
    fn write_config(dir: &Path, body: &str) {
        std::fs::write(dir.join("cortex.toml"), body).expect("write cortex.toml");
    }

    // With no `--agent` flag the run uses the default agent; an explicit valid name resolves to
    // that persona (case-insensitively).
    #[test]
    fn resolve_agent_defaults_and_honors_explicit_names() {
        assert_eq!(resolve_agent(None), Ok(Agent::default()));
        assert_eq!(resolve_agent(Some("plan")), Ok(Agent::Plan));
        assert_eq!(resolve_agent(Some("build")), Ok(Agent::Build));
        assert_eq!(resolve_agent(Some(" General ")), Ok(Agent::General));
        assert_eq!(resolve_agent(Some("EXPLORE")), Ok(Agent::Explore));
    }

    // VAL-LLM-028 (resolution half): an unknown agent name is rejected with an error that names the
    // offending agent and hints the available agents, so the CLI can fail fast before any network.
    #[test]
    fn resolve_agent_rejects_unknown_names_with_an_actionable_error() {
        let err = resolve_agent(Some("no-such-agent")).expect_err("unknown agent must be rejected");
        let message = err.to_string();
        assert!(
            message.contains("no-such-agent"),
            "the error must name the unknown agent, got: {message}"
        );
        assert!(
            message.contains("plan") && message.contains("build") && message.contains("explore"),
            "the error must hint the available agents, got: {message}"
        );
    }

    #[test]
    fn model_overrides_maps_only_an_explicit_flag() {
        assert!(
            model_overrides(None).is_empty(),
            "no flag yields no override so lower layers decide"
        );
        let with = model_overrides(Some("anthropic/claude-sonnet-4"));
        assert_eq!(
            with.get("model").map(String::as_str),
            Some("anthropic/claude-sonnet-4")
        );
    }

    // VAL-LLM-011: with no `--model` flag and no config override, the resolved model is the
    // mission default `google/gemini-2.5-flash`.
    #[test]
    fn default_model_when_no_flag_and_no_config() {
        let (base, proj) = scratch_tree("default");
        let resolved =
            resolve_run_model(config_inputs(&base, &proj, &[], None)).expect("resolve model");
        assert_eq!(resolved.model_id, "google/gemini-2.5-flash");
    }

    // VAL-LLM-012 / VAL-LLM-013: an explicit `--model` is honored verbatim, for both the default
    // model requested explicitly and a second, distinct valid model.
    #[test]
    fn explicit_flag_is_honored_for_either_valid_model() {
        let (base, proj) = scratch_tree("explicit");
        let default = resolve_run_model(config_inputs(
            &base,
            &proj,
            &[],
            Some("google/gemini-2.5-flash"),
        ))
        .expect("resolve model");
        assert_eq!(default.model_id, "google/gemini-2.5-flash");

        let second = resolve_run_model(config_inputs(
            &base,
            &proj,
            &[],
            Some("anthropic/claude-sonnet-4"),
        ))
        .expect("resolve model");
        assert_eq!(second.model_id, "anthropic/claude-sonnet-4");
    }

    // VAL-LLM-029 (config half): a config-file `model` is honored when no `--model` flag is given.
    #[test]
    fn config_file_model_is_honored_without_a_flag() {
        let (base, proj) = scratch_tree("config");
        write_config_model(&proj, "anthropic/claude-sonnet-4");
        let resolved =
            resolve_run_model(config_inputs(&base, &proj, &[], None)).expect("resolve model");
        assert_eq!(resolved.model_id, "anthropic/claude-sonnet-4");
    }

    // VAL-LLM-029 (override half): an explicit `--model` overrides a config-file `model`.
    #[test]
    fn flag_overrides_a_config_file_model() {
        let (base, proj) = scratch_tree("override");
        write_config_model(&proj, "anthropic/claude-sonnet-4");
        let resolved = resolve_run_model(config_inputs(
            &base,
            &proj,
            &[],
            Some("google/gemini-2.5-flash"),
        ))
        .expect("resolve model");
        assert_eq!(resolved.model_id, "google/gemini-2.5-flash");
    }

    // The documented precedence chain holds end-to-end: env (`CORTEX_MODEL`) overrides the config
    // file, and a `--model` flag overrides env.
    #[test]
    fn env_overrides_config_and_flag_overrides_env() {
        let (base, proj) = scratch_tree("precedence");
        write_config_model(&proj, "anthropic/claude-sonnet-4");

        let env_only = resolve_run_model(config_inputs(
            &base,
            &proj,
            &[("CORTEX_MODEL", "openai/gpt-4o-mini")],
            None,
        ))
        .expect("resolve model");
        assert_eq!(
            env_only.model_id, "openai/gpt-4o-mini",
            "env overrides the config file"
        );

        let flag_wins = resolve_run_model(config_inputs(
            &base,
            &proj,
            &[("CORTEX_MODEL", "openai/gpt-4o-mini")],
            Some("google/gemini-2.5-flash"),
        ))
        .expect("resolve model");
        assert_eq!(
            flag_wins.model_id, "google/gemini-2.5-flash",
            "the --model flag overrides env"
        );
    }

    // VAL-LLM-014 (resolution half): an unknown model id is preserved verbatim (OpenRouter is a
    // passthrough) with no catalogue metadata, so the offending id reaches the model-id check (or
    // the provider) rather than being silently coerced to the default.
    #[test]
    fn unknown_model_is_preserved_verbatim() {
        let (base, proj) = scratch_tree("unknown");
        let resolved =
            resolve_run_model(config_inputs(&base, &proj, &[], Some("not-a-real-model")))
                .expect("resolve model");
        assert_eq!(resolved.model_id, "not-a-real-model");
        assert!(
            resolved.info.is_none(),
            "an unknown id carries no catalogue metadata"
        );
    }

    // VAL-LLM-014 (validation half): a bare model id with no provider prefix is rejected fast with
    // an actionable error that names the offending model and reads as unknown/invalid.
    #[test]
    fn bare_model_id_is_rejected_with_an_actionable_error() {
        let err = validate_model_id("not-a-real-model").expect_err("a bare id must be rejected");
        assert!(
            err.contains("not-a-real-model"),
            "the error must name the offending model, got: {err}"
        );
        assert!(
            err.to_lowercase().contains("unknown") && err.contains("provider/model"),
            "the error must read as unknown and explain the expected format, got: {err}"
        );
        // An empty provider or empty model half is equally malformed.
        assert!(validate_model_id("/gemini-2.5-flash").is_err());
        assert!(validate_model_id("google/").is_err());
        assert!(validate_model_id("").is_err());
    }

    // VAL-CROSS-006 (decision half): a session's stored model governs a resume only when no
    // explicit `--model` was passed; the flag always wins, and a blank/absent stored model leaves
    // the config-resolved model in place.
    #[test]
    fn session_model_override_honors_a_stored_model_only_without_an_explicit_flag() {
        // No flag + a stored model -> the stored model overrides (trimmed).
        assert_eq!(
            session_model_override(false, Some("anthropic/claude-sonnet-4")),
            Some("anthropic/claude-sonnet-4".to_owned())
        );
        assert_eq!(
            session_model_override(false, Some("  google/gemini-2.5-flash  ")),
            Some("google/gemini-2.5-flash".to_owned()),
            "the stored model is trimmed"
        );
        // An explicit `--model` always wins, so the stored model is ignored.
        assert_eq!(
            session_model_override(true, Some("anthropic/claude-sonnet-4")),
            None
        );
        // A blank or absent stored model leaves the config-resolved model in place.
        assert_eq!(session_model_override(false, None), None);
        assert_eq!(session_model_override(false, Some("   ")), None);
    }

    // Well-formed `provider/model` ids pass validation, including ones unknown to the embedded
    // catalogue, so the OpenRouter passthrough is preserved for models newer than the snapshot.
    #[test]
    fn well_formed_model_ids_pass_validation() {
        for id in [
            "google/gemini-2.5-flash",
            "anthropic/claude-sonnet-4",
            "some/brand-new-model-2099",
            "openrouter/auto",
        ] {
            assert!(
                validate_model_id(id).is_ok(),
                "a fully-qualified id must pass validation: {id}"
            );
        }
    }

    // VAL-LLM-032 (override plumbing): a nested `[provider.openrouter] base_url` is surfaced as the
    // endpoint override, so a run can be retargeted at an alternate (e.g. unreachable) endpoint.
    #[test]
    fn base_url_override_from_nested_provider_table() {
        let (base, proj) = scratch_tree("base-url-nested");
        write_config(
            &proj,
            "[provider.openrouter]\nbase_url = \"http://127.0.0.1:3250\"\n",
        );
        let run_config =
            resolve_run_config(config_inputs(&base, &proj, &[], None)).expect("resolve");
        assert_eq!(
            run_config.base_url.as_deref(),
            Some("http://127.0.0.1:3250")
        );
    }

    // The flat `openrouter_base_url` key is what `CORTEX_OPENROUTER_BASE_URL` maps to via the
    // standard `CORTEX_<KEY>` env mapping, so an env override retargets the endpoint.
    #[test]
    fn base_url_override_from_env_mapped_flat_key() {
        let (base, proj) = scratch_tree("base-url-env");
        let run_config = resolve_run_config(config_inputs(
            &base,
            &proj,
            &[("CORTEX_OPENROUTER_BASE_URL", "http://127.0.0.1:3260")],
            None,
        ))
        .expect("resolve");
        assert_eq!(
            run_config.base_url.as_deref(),
            Some("http://127.0.0.1:3260")
        );
    }

    // The env-mapped flat key is authoritative: it wins over a nested config-file value.
    #[test]
    fn flat_base_url_override_wins_over_nested() {
        let (base, proj) = scratch_tree("base-url-precedence");
        write_config(
            &proj,
            "[provider.openrouter]\nbase_url = \"http://127.0.0.1:3250\"\n",
        );
        let run_config = resolve_run_config(config_inputs(
            &base,
            &proj,
            &[("CORTEX_OPENROUTER_BASE_URL", "http://127.0.0.1:3299")],
            None,
        ))
        .expect("resolve");
        assert_eq!(
            run_config.base_url.as_deref(),
            Some("http://127.0.0.1:3299")
        );
    }

    // With nothing configured the override is absent, so the catalogue default base URL is used.
    #[test]
    fn no_base_url_override_by_default() {
        let (base, proj) = scratch_tree("base-url-none");
        let run_config =
            resolve_run_config(config_inputs(&base, &proj, &[], None)).expect("resolve");
        assert!(run_config.base_url.is_none());
        assert!(run_config.api_key.is_none());
    }

    // A nested `[provider.openrouter] api_key` is surfaced as the config-layer credential (the
    // lowest-precedence credential source, below the env var and the 0600 store).
    #[test]
    fn api_key_from_nested_provider_table() {
        let (base, proj) = scratch_tree("api-key-nested");
        write_config(&proj, "[provider.openrouter]\napi_key = \"from-config\"\n");
        let run_config =
            resolve_run_config(config_inputs(&base, &proj, &[], None)).expect("resolve");
        assert_eq!(run_config.api_key.as_deref(), Some("from-config"));
    }

    // A flat `api_key` key is also accepted as the config-layer credential.
    #[test]
    fn api_key_from_flat_key() {
        let (base, proj) = scratch_tree("api-key-flat");
        write_config(&proj, "api_key = \"flat-config-key\"\n");
        let run_config =
            resolve_run_config(config_inputs(&base, &proj, &[], None)).expect("resolve");
        assert_eq!(run_config.api_key.as_deref(), Some("flat-config-key"));
    }

    #[test]
    fn strips_single_trailing_lf() {
        assert_eq!(strip_one_trailing_newline("hi\n"), "hi");
    }

    #[test]
    fn strips_single_trailing_crlf() {
        assert_eq!(strip_one_trailing_newline("hi\r\n"), "hi");
    }

    #[test]
    fn keeps_interior_and_extra_newlines() {
        assert_eq!(strip_one_trailing_newline("a\nb\n\n"), "a\nb\n");
        assert_eq!(strip_one_trailing_newline("no newline"), "no newline");
    }

    #[test]
    fn empty_stdin_is_no_input() {
        assert_eq!(normalize_stdin(""), None);
        assert_eq!(normalize_stdin("\n"), None);
        assert_eq!(normalize_stdin("\r\n"), None);
    }

    #[test]
    fn nonempty_stdin_is_normalized() {
        assert_eq!(
            normalize_stdin("ping from stdin\n"),
            Some("ping from stdin".to_owned())
        );
    }

    #[test]
    fn unicode_stdin_is_preserved_without_corruption() {
        assert_eq!(
            normalize_stdin("日本語-café-🚀\n"),
            Some("日本語-café-🚀".to_owned())
        );
    }

    // VAL-LLM-025 plumbing: a positional instruction and piped (non-terminal) stdin are combined,
    // with the piped content first (the "padding"/context) and the instruction after, so a large
    // piped input reaches the model alongside the short instruction.
    #[test]
    fn argument_is_combined_with_piped_stdin() {
        let resolved = resolve_prompt(Some("FROM_ARG".to_owned()), false, piped("FROM_STDIN\n"))
            .expect("resolve");
        assert_eq!(resolved, Some("FROM_STDIN\n\nFROM_ARG".to_owned()));
    }

    #[test]
    fn argument_alone_when_no_piped_stdin() {
        // Non-terminal but empty stdin: nothing to combine, so the argument is the prompt.
        let resolved =
            resolve_prompt(Some("FROM_ARG".to_owned()), false, piped("")).expect("resolve");
        assert_eq!(resolved, Some("FROM_ARG".to_owned()));
    }

    #[test]
    fn interactive_terminal_stdin_is_not_read_with_an_argument() {
        // A terminal is never read (it would block); the argument stands alone.
        let resolved = resolve_prompt(Some("FROM_ARG".to_owned()), true, piped("anything\n"))
            .expect("resolve");
        assert_eq!(resolved, Some("FROM_ARG".to_owned()));
    }

    #[test]
    fn stdin_used_when_no_argument() {
        let resolved = resolve_prompt(None, false, piped("from stdin\n")).expect("resolve");
        assert_eq!(resolved, Some("from stdin".to_owned()));
    }

    #[test]
    fn empty_stdin_without_argument_is_none() {
        let resolved = resolve_prompt(None, false, piped("")).expect("resolve");
        assert_eq!(resolved, None);
    }

    #[test]
    fn interactive_terminal_without_argument_is_none() {
        let resolved = resolve_prompt(None, true, piped("anything\n")).expect("resolve");
        assert_eq!(resolved, None);
    }

    #[test]
    fn empty_argument_with_no_piped_stdin_is_preserved_as_explicit_prompt() {
        let resolved = resolve_prompt(Some(String::new()), false, piped("")).expect("resolve");
        assert_eq!(resolved, Some(String::new()));
    }

    // VAL-RUN-004 plumbing: the operational idle-timeout value is parsed only for a positive
    // integer; an absent, blank, zero, or unparseable value leaves the transport default in place.
    #[test]
    fn parse_idle_timeout_accepts_only_positive_integers() {
        assert_eq!(
            parse_idle_timeout_ms(Some("50")),
            Some(Duration::from_millis(50))
        );
        assert_eq!(
            parse_idle_timeout_ms(Some("  5 ")),
            Some(Duration::from_millis(5)),
            "surrounding whitespace is trimmed"
        );
        assert_eq!(
            parse_idle_timeout_ms(None),
            None,
            "absent value is no override"
        );
        assert_eq!(
            parse_idle_timeout_ms(Some("")),
            None,
            "blank value is no override"
        );
        assert_eq!(parse_idle_timeout_ms(Some("0")), None, "zero is rejected");
        assert_eq!(
            parse_idle_timeout_ms(Some("not-a-number")),
            None,
            "an unparseable value is no override"
        );
        assert_eq!(
            parse_idle_timeout_ms(Some("-5")),
            None,
            "a negative value is no override"
        );
    }

    // The compaction policy is derived from the resolved model's context window: a known model
    // uses its catalogued window, an unknown model falls back to the engine default, and both
    // yield a usable (non-zero) input budget.
    #[test]
    fn compaction_policy_tracks_the_model_context_window() {
        let known = providers::resolve_model(Some("google/gemini-2.5-flash"));
        let policy = compaction_policy(&known);
        assert_eq!(policy.context_limit_tokens, 1_048_576);
        assert!(policy.input_budget() > 0);

        let unknown = providers::resolve_model(Some("some/brand-new-model-2099"));
        let fallback = compaction_policy(&unknown);
        assert_eq!(
            fallback.context_limit_tokens,
            engine::DEFAULT_CONTEXT_LIMIT_TOKENS
        );
        assert!(fallback.input_budget() > 0);
    }

    // The context-window override token is parsed only as a positive integer; an absent, blank,
    // zero, or unparseable value leaves the model-derived window in place.
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

    #[test]
    fn jsonl_shorthand_overrides_format() {
        assert_eq!(
            effective_format(true, OutputFormat::Text),
            OutputFormat::Jsonl
        );
        assert_eq!(
            effective_format(false, OutputFormat::Json),
            OutputFormat::Json
        );
        assert_eq!(
            effective_format(false, OutputFormat::Text),
            OutputFormat::Text
        );
    }

    /// A completed-turn outcome with assistant text and integer usage, mirroring a real turn.
    fn sample_outcome() -> RunOutcome {
        RunOutcome {
            message: llm::Message::assistant("Hello! How can I help you today?"),
            usage: Some(llm::Usage {
                input_tokens: 9,
                output_tokens: 8,
                total_tokens: 17,
                cost: Some(0.0000056),
            }),
            finish_reason: llm::FinishReason::Stop,
            steps: 1,
            error: None,
        }
    }

    // VAL-LLM-005: `--format json` emits exactly one strict JSON value with a single trailing
    // newline and no other framing bytes, so a strict parser accepts it whole.
    #[test]
    fn json_document_is_a_single_strict_value_with_one_trailing_newline() {
        let rendered = render_json_document(
            "google/gemini-2.5-flash",
            "build",
            "ses_x",
            &[],
            &sample_outcome(),
            None,
            Vec::new(),
        )
        .expect("render");
        assert!(rendered.ends_with('\n'), "document must end with a newline");
        assert_eq!(
            rendered.matches('\n').count(),
            1,
            "the document is a single line (no interleaved newlines)"
        );
        let value: serde_json::Value =
            serde_json::from_str(&rendered).expect("strict JSON parse of the whole document");
        assert!(value.is_object(), "the document is a single JSON object");
    }

    // VAL-LLM-004 / VAL-LLM-006 / VAL-LLM-018: the document carries the assistant role, the model
    // text as content, the model id, the re-usable session id, and a usage object with integer
    // input/output token counts.
    #[test]
    fn json_document_carries_assistant_role_content_model_session_and_integer_usage() {
        let rendered = render_json_document(
            "google/gemini-2.5-flash",
            "plan",
            "ses_abc123",
            &[],
            &sample_outcome(),
            None,
            Vec::new(),
        )
        .expect("render");
        let value: serde_json::Value = serde_json::from_str(&rendered).expect("json");
        assert_eq!(value["role"], "assistant");
        assert_eq!(value["content"], "Hello! How can I help you today?");
        assert_eq!(value["model"], "google/gemini-2.5-flash");
        assert_eq!(
            value["agent"], "plan",
            "the document surfaces the selected agent"
        );
        assert_eq!(
            value["session"], "ses_abc123",
            "the document surfaces the re-usable session id"
        );
        assert_eq!(value["finish_reason"], "stop");
        // VAL-RUN-003: the document reports a bounded step/iteration count (<= MAX_STEPS = 25).
        let steps = value["steps"].as_u64().expect("steps is an integer");
        assert_eq!(steps, 1, "a single-round-trip turn reports one step");
        assert!(
            steps <= 25,
            "the reported step count never exceeds MAX_STEPS"
        );
        assert_eq!(value["usage"]["input_tokens"].as_u64(), Some(9));
        assert_eq!(value["usage"]["output_tokens"].as_u64(), Some(8));
        assert!(
            value["usage"]["input_tokens"].is_u64() && value["usage"]["output_tokens"].is_u64(),
            "token counts must be integers"
        );
    }

    // VAL-LLM-031: when present, the document carries a `session_usage` cumulative total alongside
    // the per-turn `usage`, with integer token counts and a numeric cost, so a machine consumer can
    // read the running session-level figure.
    #[test]
    fn json_document_carries_session_usage_alongside_per_turn_usage() {
        let session_usage = UsageTotals {
            input_tokens: 248,
            output_tokens: 16,
            total_tokens: 264,
            cost: Some(0.000_079_4),
        };
        let rendered = render_json_document(
            "google/gemini-2.5-flash",
            "build",
            "ses_cumulative",
            &[],
            &sample_outcome(),
            Some(session_usage),
            Vec::new(),
        )
        .expect("render");
        let value: serde_json::Value = serde_json::from_str(&rendered).expect("json");

        // The per-turn usage stays present (this turn's figures).
        assert_eq!(value["usage"]["input_tokens"].as_u64(), Some(9));
        assert_eq!(value["usage"]["output_tokens"].as_u64(), Some(8));

        // The cumulative session total is carried as integers plus a numeric cost, and is >= the
        // per-turn figure (it includes this turn and every prior turn).
        assert_eq!(value["session_usage"]["input_tokens"].as_u64(), Some(248));
        assert_eq!(value["session_usage"]["output_tokens"].as_u64(), Some(16));
        assert_eq!(value["session_usage"]["total_tokens"].as_u64(), Some(264));
        assert!(
            value["session_usage"]["input_tokens"].is_u64()
                && value["session_usage"]["output_tokens"].is_u64()
                && value["session_usage"]["total_tokens"].is_u64(),
            "session token counts must be integers"
        );
        let session_cost = value["session_usage"]["cost"]
            .as_f64()
            .expect("session cost is a number");
        assert!(session_cost > 0.0, "a paid session reports a positive cost");
        assert!(
            value["session_usage"]["total_tokens"].as_u64().unwrap()
                >= value["usage"]["total_tokens"].as_u64().unwrap(),
            "the session total must be at least this turn's usage"
        );
    }

    // The usage object is optional: when the provider reports none, it is omitted yet the document
    // stays a valid, strictly-parseable JSON object with the assistant role, content, and session.
    #[test]
    fn json_document_omits_usage_when_absent_but_stays_valid() {
        let mut outcome = sample_outcome();
        outcome.usage = None;
        let rendered =
            render_json_document("m", "general", "ses_y", &[], &outcome, None, Vec::new())
                .expect("render");
        let value: serde_json::Value = serde_json::from_str(&rendered).expect("json");
        assert_eq!(value["role"], "assistant");
        assert_eq!(value["content"], "Hello! How can I help you today?");
        assert_eq!(value["agent"], "general");
        assert_eq!(value["session"], "ses_y");
        assert!(value.get("usage").is_none(), "absent usage is omitted");
        assert!(
            value.get("session_usage").is_none(),
            "absent session usage is omitted"
        );
    }

    // The leading `--jsonl` session event is a typed, independently-parseable object carrying the
    // re-usable session id, so a machine consumer can capture it before the runner's events stream.
    #[test]
    fn jsonl_session_line_is_typed_and_carries_the_agent_and_session_id() {
        let tools = vec!["read".to_owned(), "apply_patch".to_owned()];
        let line = render_session_line("plan", "ses_seed42", &tools).expect("render");
        let value: serde_json::Value = serde_json::from_str(&line).expect("json");
        assert_eq!(value["type"], "session");
        assert_eq!(value["agent"], "plan");
        assert_eq!(value["session"], "ses_seed42");
        // VAL-TOOLS-019/020: the session event advertises the per-turn tool set.
        assert_eq!(
            value["tools"],
            serde_json::json!(["read", "apply_patch"]),
            "the jsonl session event advertises the materialized tool set"
        );
    }

    // VAL-TOOLS-019/020: the `--format json` document advertises the per-turn (model-aware) tool
    // set so a machine consumer can observe which tools were materialized for the turn.
    #[test]
    fn json_document_advertises_the_per_turn_tool_set() {
        let gpt_tools = vec!["read".to_owned(), "apply_patch".to_owned()];
        let rendered = render_json_document(
            "openai/gpt-4o-mini",
            "build",
            "ses_gpt",
            &gpt_tools,
            &sample_outcome(),
            None,
            Vec::new(),
        )
        .expect("render");
        let value: serde_json::Value = serde_json::from_str(&rendered).expect("json");
        let tools = value["tools"].as_array().expect("tools is an array");
        let names: Vec<&str> = tools.iter().filter_map(serde_json::Value::as_str).collect();
        assert!(names.contains(&"apply_patch"), "got: {names:?}");
        assert!(!names.contains(&"edit"), "got: {names:?}");
        assert!(!names.contains(&"write"), "got: {names:?}");
    }

    // `session_selection` maps the mutually-exclusive flags: an explicit id wins, `--continue`
    // attaches to the latest, and neither yields a fresh session.
    #[test]
    fn session_selection_maps_flags() {
        assert!(matches!(
            session_selection(Some("val-1".to_owned()), false),
            SessionSelection::Explicit(id) if id == "val-1"
        ));
        // An explicit id wins even if `--continue` were somehow also set (clap forbids this).
        assert!(matches!(
            session_selection(Some("val-2".to_owned()), true),
            SessionSelection::Explicit(id) if id == "val-2"
        ));
        assert!(matches!(
            session_selection(None, true),
            SessionSelection::Continue
        ));
        assert!(matches!(
            session_selection(None, false),
            SessionSelection::New
        ));
    }

    // VAL-LLM-007 / VAL-LLM-008: the JSONL stream renders >=2 independently-parseable lines, each
    // carrying a stable `type` discriminator; at least one text event and a terminal usage/done are
    // present, the stream ends with the terminal event, and concatenated text deltas reconstruct
    // the answer.
    #[test]
    fn jsonl_lines_are_typed_terminate_with_done_and_reconstruct_the_answer() {
        let events = [
            RunnerEvent::TextDelta {
                text: "Hello".to_owned(),
            },
            RunnerEvent::TextDelta {
                text: ", world!".to_owned(),
            },
            RunnerEvent::Usage(llm::Usage {
                input_tokens: 5,
                output_tokens: 3,
                total_tokens: 8,
                cost: None,
            }),
            RunnerEvent::Done {
                reason: llm::FinishReason::Stop,
                steps: 1,
            },
        ];
        let lines: Vec<String> = events
            .iter()
            .map(|event| render_event_line(event).expect("render"))
            .collect();
        assert!(lines.len() >= 2, "incremental events span >=2 lines");

        let mut reconstructed = String::new();
        let mut types = Vec::new();
        for line in &lines {
            let value: serde_json::Value =
                serde_json::from_str(line).expect("each line parses as JSON");
            let kind = value["type"]
                .as_str()
                .expect("every event carries a `type` discriminator")
                .to_owned();
            if kind == "text_delta" {
                reconstructed.push_str(value["text"].as_str().expect("text fragment"));
            }
            types.push(kind);
        }
        assert_eq!(
            reconstructed, "Hello, world!",
            "concatenated text deltas reconstruct the answer"
        );
        assert!(
            types.iter().any(|t| t == "text_delta"),
            "at least one text event present"
        );
        assert!(
            types.iter().any(|t| t == "usage" || t == "done"),
            "a usage/final/done event present"
        );
        assert_eq!(
            types.last().map(String::as_str),
            Some("done"),
            "the stream ends with the terminal event"
        );

        // VAL-RUN-003: the terminal Done line reports a bounded step count (<= MAX_STEPS = 25).
        let done_value: serde_json::Value =
            serde_json::from_str(lines.last().expect("a terminal line")).expect("json");
        let steps = done_value["steps"]
            .as_u64()
            .expect("done carries a step count");
        assert!(
            steps <= 25,
            "the reported step count never exceeds MAX_STEPS"
        );
    }
}
