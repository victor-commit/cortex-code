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

use std::collections::{BTreeMap, BTreeSet};
use std::io::{self, IsTerminal, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use checkpoints::TurnRecorder;
use db::{Database, DbPathInputs, UsageTotals};
use engine::{
    CompactionPolicy, EnvironmentBlock, HistoryMessage, PartLifecycleEvent, PartLifecycleTracker,
    RunOutcome, RunnerEvent, SessionRunner, SystemContext, ToolDispatcher, Turn,
    build_assistant_message_parts, project_history,
};
use errors::{Diagnostic, Exit};
use events::{Event, EventBus};
use futures_util::future::BoxFuture;
use llm::{LlmError, LlmRequest, LlmStream, MessageContentPart, Provider, ToolDef};
use protocol::{MessagePart, MessagePartBody, ToolResultPart};
use providers::ResolvedModel;
use redaction::guard_outbound;
use regex::Regex;
use serde::Serialize;
use settings::{ConfigError, ConfigInputs, EffectiveConfig};

use crate::cli::{OutputFormat, RunArgs};
use crate::custom_agents;
use crate::custom_modes;
use crate::permission;
use crate::permission_store;
use crate::plugin_bridge::{
    CommandBeforeOutcome, CommandInvocation, PluginState, WorkspaceAdapterDoc,
};
use crate::style;
use crate::tooling::RegistryDispatcher;

/// The outbound operation named in a `net-disabled` refusal raised before the turn starts.
const OUTBOUND_OPERATION: &str = "LLM turn";

/// Execute a single headless `cortex run` turn.
pub fn run(args: RunArgs, bus: Arc<EventBus>, pure_plugins: bool) -> Exit {
    if args.command.is_some() {
        return run_command_path(args, bus, pure_plugins);
    }
    if !crate::feature_gate::network_enabled() {
        return crate::feature_gate::network_unavailable("run");
    }

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
    let structured_schema = match parse_structured_schema(args.schema.as_deref(), format) {
        Ok(schema) => schema,
        Err(message) => {
            eprintln!("cortex run: {message}");
            return Exit::Usage;
        }
    };

    // Resolve the local config inputs early: custom agent discovery uses the same working directory
    // and config layers as the eventual run, but it stays local and happens before any network work.
    let config_inputs = match ConfigInputs::from_process(model_overrides(args.model.as_deref())) {
        Ok(config_inputs) => config_inputs,
        Err(err) => {
            eprintln!("cortex run: cannot determine the working directory: {err}");
            return Exit::Failure;
        }
    };

    let discovered_agents = custom_agents::discover(&config_inputs);
    for warning in &discovered_agents.warnings {
        eprintln!("cortex run: {warning}");
    }
    let discovered_modes = custom_modes::discover(&config_inputs);
    for warning in &discovered_modes.warnings {
        eprintln!("cortex run: {warning}");
    }
    let discovered_skills = skills::discover(&config_inputs);
    for warning in &discovered_skills.warnings {
        eprintln!("cortex run: {warning}");
    }
    let discovered_skill_list = discovered_skills.ordered_skills();
    let project_memory = memory::discover(&config_inputs);
    for warning in project_memory.warnings() {
        eprintln!("cortex run: {warning}");
    }
    if project_memory.net_blocked() {
        eprintln!(
            "cortex run: network is disabled by CORTEX_NO_NET=1; remote instruction URLs cannot be fetched"
        );
        return Exit::NetDisabled;
    }
    let dynamic_memory_tracker =
        memory::DynamicMemoryTracker::new(config_inputs.clone(), &project_memory);
    let project_memory_block = project_memory.render();

    let configured_agent = if args.agent.is_none() {
        match configured_agent_name(&config_inputs) {
            Ok(agent) => agent,
            Err(err) => {
                eprintln!("cortex run: {err}");
                return Exit::Config;
            }
        }
    } else {
        None
    };
    let requested_agent = args.agent.as_deref().or(configured_agent.as_deref());
    let configured_mode = if args.mode.is_none() {
        match configured_mode_name(&config_inputs) {
            Ok(mode) => mode,
            Err(err) => {
                eprintln!("cortex run: {err}");
                return Exit::Config;
            }
        }
    } else {
        None
    };
    let requested_mode = args.mode.as_deref().or(configured_mode.as_deref());

    // Resolve the agent persona before any network consideration so an unknown agent fails fast
    // with an actionable, agent-naming error and a non-zero exit, never touching the network or the
    // durable store and never writing an assistant answer to stdout. When `--agent` is absent, the
    // layered config's `agent` key supplies the default persona.
    let agent = match custom_agents::resolve(requested_agent, &discovered_agents) {
        Ok(agent) => agent,
        Err(err) => {
            eprintln!(
                "cortex run: {err}; select one with --agent <name> (for example --agent build)"
            );
            return Exit::Failure;
        }
    };
    let mode = match custom_modes::resolve(requested_mode, &discovered_modes) {
        Ok(mode) => mode,
        Err(err) => {
            eprintln!("cortex run: {err}; select one with --mode <name> (for example --mode plan)");
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

    let subagent_config_inputs = config_inputs.clone();
    let mut run_config = match resolve_run_config(config_inputs.clone()) {
        Ok(run_config) => run_config,
        Err(err) => {
            eprintln!("cortex run: {err}");
            return Exit::Config;
        }
    };

    // Open the durable store and resolve the session this turn belongs to before the outbound
    // guard, so even a fail-closed network refusal can be correlated to a durable session and
    // written to the audit sink without touching credentials or sockets.
    let (db, db_path) = match open_database() {
        Ok(opened) => opened,
        Err(exit) => return exit,
    };
    let session_id = match resolve_session(&db, session_selection(args.session, args.r#continue)) {
        Ok(id) => id,
        Err(exit) => return exit,
    };
    let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    // Report image normalization (downscale/bounds) before the network guard or any provider work,
    // so the size/dimension change a multimodal turn will apply is observable even when the turn
    // later fails closed (no network/credential). This only reads local files; it opens no socket.
    emit_image_normalization_diagnostics(&extract_message_content_parts(&prompt_message_parts(
        &prompt, &cwd,
    )));
    // Link this run's session to the project that owns the working directory, so the session keeps
    // its project identity across restarts. Best-effort: a project-link failure must not abort an
    // otherwise-valid run.
    if let Ok(project) = crate::project_cmd::resolve_project(&db, &cwd) {
        let _ = db.sessions().set_project(&session_id, Some(&project.id));
    }
    let audit_sink = Arc::new(
        audit::AuditSink::from_env_or_cwd(&session_id, &cwd)
            .with_surface_and_source("run", "headless"),
    );

    // Fail closed on the network kill-switch before resolving a credential or opening any socket,
    // so a refusal never touches the network even when a key is present. The refusal itself is a
    // blocked network attempt and is durable in the same structured audit sink as successful runs.
    if let Err(diag) = guard_outbound(OUTBOUND_OPERATION) {
        let endpoint = run_config
            .provider_config
            .resolve_stream_endpoint(
                run_config.model.provider_id.as_str(),
                run_config.model.model_id.as_str(),
            )
            .unwrap_or_else(|_| {
                format!(
                    "{}/chat/completions",
                    providers::openrouter_base_url().trim_end_matches('/')
                )
            });
        let host = audit::host_from_endpoint(&endpoint).unwrap_or_default();
        audit_sink.record_network_call(
            "llm",
            Some(run_config.model.provider_id.as_str()),
            Some(&run_config.model.model_id),
            "POST",
            &host,
            &endpoint,
            None,
            diag.kind(),
        );
        eprintln!("cortex run: {diag}");
        return diag.exit();
    }

    // Load configured local plugins through the external JavaScript runtime contract after the
    // audit session is ready but before provider credentials are resolved. A configured plugin is
    // fail-closed: missing Bun/Node/Deno, a missing file, or a load-time throw stops the run with
    // an attributed error rather than silently continuing without hooks.
    let plugin_state = match PluginState::load_or_disabled_for_surface_with_config(
        &run_config.plugins,
        pure_plugins,
        "run",
        run_config.resolved_config.clone(),
    ) {
        Ok(state) => state,
        Err(err) => {
            eprintln!("cortex run: {err}");
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
    // overrides), so a per-agent rule gates the turn (VAL-PERM-009); a project-configured default
    // agent is active in the same way as a CLI-selected one, so its mode restrictions and scoped
    // permission rules govern the turn across surfaces.
    let active_agent = Some(agent.name());
    let effective_base_agent = mode
        .as_ref()
        .and_then(custom_modes::ActiveMode::base_agent)
        .unwrap_or_else(|| agent.base_agent());
    let builtin_agent_rules = permission::builtin_agent_permission_rules(effective_base_agent);
    let builtin_hard_denies = builtin_agent_rules.restriction_rules.clone();
    let mut post_save_permission_rules = agent.permissions().to_vec();
    if let Some(mode) = &mode {
        post_save_permission_rules.extend(mode.permissions().iter().cloned());
    }
    post_save_permission_rules.extend(builtin_agent_rules.restriction_rules);

    // Resolve the project's trust level (the layered config's `trust` key) into trust deny rules.
    // These are installed as hard denies in the engine, so an untrusted project blocks the
    // mutating/exec tools before saved decisions are consulted (VAL-CROSS-049). A trusted project
    // contributes nothing. A config error fails fast with the config exit code.
    let trust_rules = match policy::resolve_trust(&config_inputs) {
        Ok(level) => policy::trust_ruleset(level).rules().to_vec(),
        Err(err) => {
            eprintln!("cortex run: {err}");
            return Exit::Config;
        }
    };

    // Build the headless permission engine from every source `cortex run` honors: config-file
    // `[[permission]]` layers (agent-scoped when an `--agent` is active) as the BASE ruleset, CLI
    // allow/deny rules appended ON TOP, previously saved cross-process decisions, and
    // `--permission-allow-save` specs. Built-in read-only mode restrictions are also attached as
    // hard denies, so a saved allow cannot bypass plan/explore mutation blocks. The engine is
    // attached whenever any source is non-empty; with none of them the run stays ungated.
    let mut effective_cli_permission_rules = plugin_state.permission_rules();
    effective_cli_permission_rules.extend(cli_permission_rules);
    let prepared_permissions =
        match permission::prepare_headless_permissions_with_builtin_rules_and_override(
            &config_inputs,
            active_agent,
            builtin_agent_rules.base_rules,
            effective_cli_permission_rules,
            save_specs,
            post_save_permission_rules,
            builtin_hard_denies,
            trust_rules,
            loaded_decisions,
            bus.clone(),
            plugin_state.permission_override(),
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
    let embedded_catalogue = providers::Catalogue::embedded();

    let agent_model_selected = if !explicit_model {
        if let Some(model) = agent.model() {
            run_config.model = match run_config
                .provider_config
                .resolve_model(embedded_catalogue, Some(model))
            {
                Ok(model) => model,
                Err(err) => {
                    eprintln!("cortex run: {err}");
                    return Exit::Config;
                }
            };
            true
        } else {
            false
        }
    } else {
        false
    };

    // Honor a model persisted with the session (chosen interactively via the TUI `/models` picker)
    // when the caller passed no explicit `--model`: resuming the session headless then uses that
    // per-session model rather than reverting to the global default (VAL-CROSS-006). This happens
    // BEFORE provider/plugin selection, so every downstream setup step sees the final model.
    let stored_model = db
        .sessions()
        .get(&session_id)
        .ok()
        .flatten()
        .and_then(|session| session.model);
    if let Some(model) = session_model_override(
        explicit_model || agent_model_selected,
        stored_model.as_deref(),
    ) {
        if let Err(message) = validate_model_id(&model) {
            eprintln!("cortex run: {message}");
            return Exit::Failure;
        }
        run_config.model = match run_config
            .provider_config
            .resolve_model(embedded_catalogue, Some(&model))
        {
            Ok(model) => model,
            Err(err) => {
                eprintln!("cortex run: {err}");
                return Exit::Config;
            }
        };
    }

    // Reject a malformed model id locally, before any credential or socket, with an actionable
    // error naming the offending model. A well-formed but unknown id is left to the provider to
    // accept or reject upstream, preserving the OpenRouter passthrough for newer models.
    if let Err(message) = validate_model_id(&run_config.model.model_id) {
        eprintln!("cortex run: {message}");
        return Exit::Failure;
    }

    let plugin_provider = plugin_state.provider_for_model(&run_config.model.model_id);
    let selected_provider = plugin_provider
        .as_ref()
        .map(|provider| provider.provider_id())
        .unwrap_or(run_config.model.provider_id.as_str());
    if let Err(err) = policy::enforce_provider_use(&config_inputs, selected_provider) {
        eprintln!("cortex run: {err}");
        return if err.is_config() {
            Exit::Config
        } else {
            Exit::Failure
        };
    }

    // The credential resolves through env > 0600 file > config indirection for the selected
    // provider. Provider config can retarget the endpoint and attach provider-scoped options.
    let mut provider_extra_params = serde_json::Map::new();
    let (provider_base, provider_audit): (
        Arc<dyn llm::Provider>,
        Option<crate::subagent::ProviderAuditConfig>,
    ) = match plugin_provider {
        Some(provider) => {
            eprintln!(
                "cortex run: using plugin provider {} for model {}",
                provider.provider_id(),
                provider.model_id()
            );
            let auth = resolve_plugin_provider_auth(
                &plugin_state,
                &run_config.provider_config,
                provider.provider_id(),
                &audit_sink,
            );
            (
                Arc::new(
                    provider
                        .with_auth(auth)
                        .with_audit(Some(audit_sink.clone())),
                ),
                None,
            )
        }
        None => {
            let provider_id = run_config.model.provider_id.clone();
            provider_extra_params = run_config.provider_config.provider_params(&provider_id);
            match providers::resolve_runtime_provider_with_options(
                &run_config.provider_config,
                &provider_id,
                run_config.model.model_id.as_str(),
                stream_idle_timeout(),
            ) {
                Ok(resolved) => (
                    resolved.provider,
                    Some(crate::subagent::ProviderAuditConfig::new(
                        provider_id,
                        resolved.endpoint,
                    )),
                ),
                Err(error) => {
                    if let Some(config_error) = error.as_config() {
                        eprintln!("cortex run: {config_error}");
                        return Exit::Config;
                    }
                    if let Some(diagnostic) = error.as_diagnostic() {
                        return report_setup_diagnostic(diagnostic);
                    }
                    unreachable!("runtime provider error must be config or diagnostic");
                }
            }
        }
    };

    plugin_state.attach_audit(audit_sink.clone());
    plugin_state.audit_loaded_plugins();
    if let Some(subscriber) = plugin_state.event_subscriber() {
        bus.subscribe(subscriber);
    }
    let provider = match &provider_audit {
        Some(config) => {
            let audited = AuditedProvider::new(
                provider_base.clone(),
                audit_sink.clone(),
                config.provider.clone(),
                config.endpoint.clone(),
            );
            Arc::new(audited) as Arc<dyn llm::Provider>
        }
        None => provider_base.clone(),
    };
    let provider = plugin_state.wrap_provider(provider);

    if let Err(err) = db
        .sessions()
        .set_model(&session_id, Some(&run_config.model.model_id))
    {
        eprintln!("cortex run: failed to persist the session model: {err}");
        return Exit::Failure;
    }

    // Project the session's prior turns into canonical history BEFORE persisting this prompt, so
    // the new user input is sent once (as the turn's input) and not duplicated into the history.
    let history = match load_history(&db, &session_id) {
        Ok(history) => history,
        Err(exit) => return exit,
    };
    let user_parts = prompt_message_parts(&prompt, &cwd);
    let user_media_parts = extract_message_content_parts(&user_parts);
    let user_message =
        match db
            .messages()
            .create_with_parts(&session_id, "user", &prompt, &user_parts)
        {
            Ok(message) => message,
            Err(err) => {
                eprintln!("cortex run: failed to persist the prompt: {err}");
                return Exit::Failure;
            }
        };
    persist_memory_provenance(&db, &session_id, project_memory.sources());
    bus.publish(&Event::turn_started(&session_id, &user_message.id));

    // The configured MCP servers for this run, taken before the model id is moved out of the config.
    let mcp_servers = std::mem::take(&mut run_config.mcp_servers);

    // Derive the compaction policy from the resolved model's context window before the model id is
    // moved out, so an over-large turn is compacted/recovered rather than crashing (VAL-LLM-025/026).
    let resolved_model = run_config.model.clone();
    let compaction = compaction_policy(&run_config.model);
    let model_id = run_config.model.model_id;
    let small_model_id = resolve_small_model_for_turn(
        &plugin_state,
        &model_id,
        run_config
            .resolved_config
            .get("small_model")
            .and_then(serde_json::Value::as_str),
    );
    let system = render_system_context(
        &model_id,
        &agent,
        mode.as_ref(),
        &discovered_skill_list,
        project_memory_block.clone(),
    );
    let mut turn = Turn::new(&model_id, system, prompt.clone())
        .with_history(history)
        .with_user_parts(user_media_parts)
        .with_compaction(compaction)
        .with_small_model(small_model_id.clone());
    if !provider_extra_params.is_empty() {
        turn = turn.with_extra_params(provider_extra_params);
    }
    if let Some(temperature) = agent.temperature() {
        turn = turn.with_temperature(temperature);
    }

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
    let lifecycle_signal = Arc::new(LifecycleSignal::new());
    let signal_watcher = lifecycle_signal.clone();
    runtime.spawn(async move {
        signal_watcher.watch().await;
    });

    // Reserve every built-in and plugin model-visible id before connecting MCP. The manager's
    // global allocator then prevents an MCP capability from shadowing any already-materialized
    // surface tool.
    let mut reserved_mcp_tool_ids = tools::ToolRegistry::reserved_builtin_ids();
    reserved_mcp_tool_ids.extend(plugin_state.tool_names());
    reserved_mcp_tool_ids.sort();
    reserved_mcp_tool_ids.dedup();

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
            roots: vec![cwd.clone()],
            reserved_tool_ids: reserved_mcp_tool_ids,
        };
        runtime
            .block_on(async {
                tokio::select! {
                    manager = mcp::McpManager::connect_with(mcp_servers, options) => Some(manager),
                    () = lifecycle_signal.wait() => None,
                }
            })
            .map(std::sync::Arc::new)
    };
    if lifecycle_signal.is_cancelled() {
        if let Some(manager) = &mcp_manager {
            runtime.block_on(manager.shutdown());
        }
        return Exit::Failure;
    }

    // Materialize the per-turn tool set for the model's family, scoped to the process working
    // directory, so the model can read/edit the filesystem for real this turn. A GPT-family model
    // gets `apply_patch` (and loses `edit`/`write`); any other model keeps `edit`/`write`
    // (architecture §3.4 / §4).
    let todo_store: std::sync::Arc<dyn tools::TodoStore> =
        std::sync::Arc::new(crate::todo_store::DbTodoStore::new(db_path.clone()));

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

    let lsp_hooks = std::sync::Arc::new(crate::lsp_bridge::LspHooks::new(
        cwd.clone(),
        run_config.lsp_config.clone(),
    ));
    let mutation_hooks = std::sync::Arc::new(crate::format_bridge::PostMutationHooks::with_config(
        cwd.clone(),
        run_config.formatter_config.clone(),
        lsp_hooks.clone(),
    ));

    let parent_permission_cap = permission_engine
        .as_ref()
        .map(|engine| engine.parent_cap_ruleset());
    let task_catalog =
        crate::subagent::permitted_catalog(&discovered_agents, permission_engine.as_ref());

    let dispatcher =
        RegistryDispatcher::for_model(cwd.clone(), &model_id, session_id.clone(), todo_store);
    let dispatcher = dispatcher.with_audit(audit_sink.clone());
    let dispatcher = dispatcher.with_dynamic_memory(dynamic_memory_tracker);
    let dispatcher = match run_config.websearch.clone() {
        Some(config) => dispatcher.with_websearch(config),
        None => dispatcher,
    };
    let dispatcher = dispatcher
        .with_skills(discovered_skill_list.clone())
        .with_extra_tools(plugin_state.tools());
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
    // Attach environment variables: the project-scoped environment store first, then plugin
    // `shell.env` hooks (which take precedence). A value set via `cortex env set` thus reaches the
    // commands the agent runs (the bash tool), and `cortex env remove` makes it no longer appear.
    let mut shell_env = crate::env_cmd::scoped_shell_env(&cwd);
    shell_env.extend(plugin_state.shell_env());
    let dispatcher = dispatcher.with_shell_env(shell_env);
    // Attach the same real LSP manager to the read-only `lsp` navigation tool, so hover,
    // definition, references, and symbols use the live server and share its lifecycle.
    let dispatcher = dispatcher.with_lsp_service(lsp_hooks.clone());
    // Attach the connected MCP manager so its server-qualified tools are advertised and callable
    // alongside the built-in tools this turn.
    let dispatcher = match &mcp_manager {
        Some(manager) => dispatcher.with_mcp(manager.clone()),
        None => dispatcher,
    };
    let task_runner: Option<std::sync::Arc<crate::subagent::CliTaskRunner>> =
        if task_catalog.targets.is_empty() {
            None
        } else {
            Some(task_catalog.runner(crate::subagent::CliTaskRunnerConfig {
                provider: provider_base.clone(),
                provider_audit: provider_audit.clone(),
                plugin_provider: Some(std::sync::Arc::new(
                    crate::subagent::PluginProviderWrapper::new(plugin_state.clone()),
                )),
                audit: audit_sink.clone(),
                db_path: db_path.clone(),
                cwd: cwd.clone(),
                parent_session_id: session_id.clone(),
                parent_cap: parent_permission_cap,
                parent_permission_override: plugin_state.permission_override(),
                parent_model_id: model_id.clone(),
                formatter_config: run_config.formatter_config.clone(),
                lsp_config: run_config.lsp_config.clone(),
                config_inputs: subagent_config_inputs,
                project_memory: project_memory_block.clone(),
                bus: bus.clone(),
            }))
        };
    let dispatcher = match &task_runner {
        Some(runner) => dispatcher.with_task_runner(task_catalog.targets.clone(), runner.clone()),
        None => dispatcher,
    };
    let shaped_dispatcher =
        plugin_state.tool_definition_dispatcher(&dispatcher, session_id.clone());
    let tool_definitions = shaped_dispatcher.definitions();
    let tool_names = tool_names_from(&tool_definitions);
    let workspace_adapters = plugin_state.workspace_adapters();

    let exit = runtime.block_on(async {
        let exit = match format {
            OutputFormat::Text => {
                stream_text(
                    provider.as_ref(),
                    &resolved_model,
                    &model_id,
                    agent.name(),
                    mode.as_ref().map(custom_modes::ActiveMode::name),
                    &session_id,
                    &user_message.id,
                    &db,
                    turn,
                    &shaped_dispatcher,
                    checkpoint_recorder.as_ref(),
                    &plugin_state,
                    bus.clone(),
                    lifecycle_signal.as_ref(),
                )
                .await
            }
            OutputFormat::Jsonl => {
                stream_jsonl(
                    provider.as_ref(),
                    &resolved_model,
                    &model_id,
                    agent.name(),
                    mode.as_ref().map(custom_modes::ActiveMode::name),
                    &session_id,
                    &user_message.id,
                    &tool_names,
                    &tool_definitions,
                    &db,
                    turn,
                    &shaped_dispatcher,
                    checkpoint_recorder.as_ref(),
                    &plugin_state,
                    bus.clone(),
                    lifecycle_signal.as_ref(),
                )
                .await
            }
            OutputFormat::Json => {
                emit_json(
                    provider.as_ref(),
                    &resolved_model,
                    &model_id,
                    agent.name(),
                    mode.as_ref().map(custom_modes::ActiveMode::name),
                    &session_id,
                    &user_message.id,
                    &db,
                    turn,
                    &shaped_dispatcher,
                    checkpoint_recorder.as_ref(),
                    &plugin_state,
                    bus.clone(),
                    lifecycle_signal.as_ref(),
                    structured_schema.as_ref(),
                    &workspace_adapters,
                )
                .await
            }
        };
        // Drive any in-flight background `task` subagent child turns to completion before the
        // process exits, so their transcripts persist durably and remain rejoinable by id after a
        // restart (a current-thread runtime stops polling spawned tasks once this block returns).
        if let Some(runner) = task_runner.as_ref() {
            runner.drain_background().await;
        }
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

    if exit == Exit::Success {
        maybe_generate_session_title(&db, &session_id, &prompt);
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

fn maybe_generate_session_title(db: &Database, session_id: &str, prompt: &str) {
    let Ok(Some(session)) = db.sessions().get(session_id) else {
        return;
    };
    if session
        .title
        .as_deref()
        .is_some_and(|title| !title.trim().is_empty())
    {
        return;
    }
    let Some(title) = title_from_prompt(prompt) else {
        return;
    };
    if let Err(err) = db.sessions().set_title(session_id, Some(&title)) {
        eprintln!("cortex run: failed to persist generated title: {err}");
    }
}

fn title_from_prompt(prompt: &str) -> Option<String> {
    let line = prompt
        .lines()
        .find(|line| !line.trim().is_empty())
        .map(str::trim)?;
    let words = line.split_whitespace().take(12).collect::<Vec<_>>();
    if words.is_empty() {
        return None;
    }
    let mut title = words.join(" ");
    if title.chars().count() > 80 {
        title = title.chars().take(80).collect();
    }
    Some(title)
}

fn run_command_path(args: RunArgs, bus: Arc<EventBus>, pure_plugins: bool) -> Exit {
    let config_inputs = match ConfigInputs::from_process(model_overrides(args.model.as_deref())) {
        Ok(config_inputs) => config_inputs,
        Err(err) => {
            eprintln!("cortex run: cannot determine the working directory: {err}");
            return Exit::Failure;
        }
    };
    let run_config = match resolve_run_config(config_inputs) {
        Ok(run_config) => run_config,
        Err(err) => {
            eprintln!("cortex run: {err}");
            return Exit::Config;
        }
    };
    let plugin_state = match PluginState::load_or_disabled_for_surface_with_config(
        &run_config.plugins,
        pure_plugins,
        "run",
        run_config.resolved_config.clone(),
    ) {
        Ok(state) => state,
        Err(err) => {
            eprintln!("cortex run: {err}");
            return Exit::Failure;
        }
    };
    if let Some(subscriber) = plugin_state.event_subscriber() {
        bus.subscribe(subscriber);
    }

    let command = CommandInvocation::new(
        args.command.expect("command path checked before dispatch"),
        args.command_args,
    );
    let command = match plugin_state.command_before(command) {
        CommandBeforeOutcome::Run(command) => command,
        CommandBeforeOutcome::Veto { plugin, reason } => {
            eprintln!("cortex run: plugin {plugin} vetoed command.execute.before: {reason}");
            plugin_state.dispose();
            return Exit::Failure;
        }
    };

    let exit = execute_local_command(command);
    plugin_state.dispose();
    exit
}

fn execute_local_command(command: CommandInvocation) -> Exit {
    match command.name.as_str() {
        "touch" => {
            let Some(path) = command.args.first() else {
                eprintln!("cortex run --command touch: missing path argument");
                return Exit::Usage;
            };
            let path = resolve_command_path(path);
            if let Some(parent) = path.parent()
                && let Err(err) = std::fs::create_dir_all(parent)
            {
                eprintln!("cortex run --command touch: {err}");
                return Exit::Failure;
            }
            match std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)
            {
                Ok(_) => Exit::Success,
                Err(err) => {
                    eprintln!("cortex run --command touch: {err}");
                    Exit::Failure
                }
            }
        }
        "write-marker" | "write" => {
            let Some(path) = command.args.first() else {
                eprintln!("cortex run --command write-marker: missing path argument");
                return Exit::Usage;
            };
            let content = if command.args.len() > 1 {
                command.args[1..].join(" ")
            } else {
                "command".to_owned()
            };
            let path = resolve_command_path(path);
            if let Some(parent) = path.parent()
                && let Err(err) = std::fs::create_dir_all(parent)
            {
                eprintln!("cortex run --command write-marker: {err}");
                return Exit::Failure;
            }
            match std::fs::write(&path, content) {
                Ok(()) => Exit::Success,
                Err(err) => {
                    eprintln!("cortex run --command write-marker: {err}");
                    Exit::Failure
                }
            }
        }
        "echo" => {
            println!("{}", command.args.join(" "));
            Exit::Success
        }
        other => {
            eprintln!("cortex run --command: unknown command `{other}`");
            Exit::Usage
        }
    }
}

fn resolve_command_path(path: &str) -> std::path::PathBuf {
    let path = std::path::Path::new(path);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| std::path::PathBuf::from("."))
            .join(path)
    }
}

fn render_system_context(
    model_id: &str,
    agent: &custom_agents::ActiveAgent,
    mode: Option<&custom_modes::ActiveMode>,
    skills: &[skills::Skill],
    project_memory: Option<String>,
) -> String {
    let effective_base_agent = mode
        .and_then(custom_modes::ActiveMode::base_agent)
        .unwrap_or_else(|| agent.base_agent());
    let mut system = SystemContext::for_model(model_id, EnvironmentBlock::from_process())
        .with_project_memory(project_memory)
        .with_agent(effective_base_agent)
        .render();
    if let Some(prompt) = agent.custom_prompt() {
        system.push_str("\n\nCustom agent instructions:\n");
        system.push_str(prompt);
    }
    if let Some(prompt) = mode.and_then(custom_modes::ActiveMode::prompt) {
        system.push_str("\n\nMode instructions:\n");
        system.push_str(prompt);
    }
    if let Some(hint) = skills::render_system_hint(skills) {
        system.push_str("\n\n");
        system.push_str(&hint);
    }
    system
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
                .filter_map(|message| {
                    if memory_provenance_from_content(&message.content) {
                        return None;
                    }
                    let media_parts = extract_message_content_parts(&message.parts);
                    let content = if let Some(summary) =
                        legacy_compaction_summary_for_message(&message)
                    {
                        summary
                    } else if message.content.trim().is_empty() {
                        message
                            .parts
                            .iter()
                            .filter_map(|part| match &part.body {
                                MessagePartBody::Text { text }
                                | MessagePartBody::Reasoning { text, .. } => Some(text.as_str()),
                                _ => None,
                            })
                            .collect::<Vec<_>>()
                            .join("\n")
                    } else {
                        message.content.clone()
                    };
                    Some(HistoryMessage::new(message.role, content).with_parts(media_parts))
                })
                .collect();
            Ok(project_history(&history))
        }
        Err(err) => {
            eprintln!("cortex run: failed to load session history: {err}");
            Err(Exit::Failure)
        }
    }
}

fn prompt_message_parts(prompt: &str, cwd: &Path) -> Vec<MessagePart> {
    let mut parts = vec![MessagePart::text(prompt.to_owned())];
    parts.extend(markdown_media_message_parts(prompt, cwd));
    parts
}

fn markdown_media_message_parts(prompt: &str, cwd: &Path) -> Vec<MessagePart> {
    let mut seen = BTreeSet::new();
    let mut parts = Vec::new();
    for captures in markdown_image_link_regex().captures_iter(prompt) {
        let Some(raw_target) = captures.get(1).map(|capture| capture.as_str()) else {
            continue;
        };
        let Some((path, media_type)) = resolve_media_reference(raw_target, cwd) else {
            continue;
        };
        let key = format!("{media_type}|{}", path.display());
        if !seen.insert(key) {
            continue;
        }
        parts.push(MessagePart {
            id: String::new(),
            created_at: None,
            body: MessagePartBody::File {
                path: path.to_string_lossy().into_owned(),
                media_type: Some(media_type),
                source: Some("markdown".to_owned()),
            },
        });
    }
    parts
}

fn markdown_image_link_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| Regex::new(r"!\[[^\]]*\]\(([^)]+)\)").expect("valid image-link regex"))
}

fn resolve_media_reference(target: &str, cwd: &Path) -> Option<(PathBuf, String)> {
    let mut value = target.trim();
    if value.len() >= 2 && value.starts_with('<') && value.ends_with('>') {
        value = &value[1..value.len() - 1];
    }
    if value.chars().any(char::is_whitespace) {
        value = value.split_whitespace().next().unwrap_or(value);
    }
    if value.is_empty()
        || value.starts_with("http://")
        || value.starts_with("https://")
        || value.starts_with("data:")
        || value.starts_with('#')
    {
        return None;
    }
    let path = Path::new(value);
    let resolved = if path.is_absolute() {
        path.to_path_buf()
    } else {
        cwd.join(path)
    };
    if !resolved.is_file() {
        return None;
    }
    let canonical = std::fs::canonicalize(&resolved).unwrap_or(resolved);
    let media_type = media_type_for_path(&canonical)?;
    Some((canonical, media_type.to_owned()))
}

fn media_type_for_path(path: &Path) -> Option<&'static str> {
    let extension = path.extension()?.to_str()?.trim().to_ascii_lowercase();
    match extension.as_str() {
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "gif" => Some("image/gif"),
        "webp" => Some("image/webp"),
        "bmp" => Some("image/bmp"),
        "svg" => Some("image/svg+xml"),
        "pdf" => Some("application/pdf"),
        _ => None,
    }
}

/// Emit a stderr diagnostic for every image attachment describing how it is normalized before
/// provider upload: oversized raster images are downscaled to within the upload bound, in-bound
/// images pass through, and an undecodable payload over the byte cap is reported as rejected. This
/// mirrors the normalization the provider seam applies so the size/dimension change is observable.
fn emit_image_normalization_diagnostics(parts: &[MessageContentPart]) {
    for part in parts {
        let (media_type, path) = match part {
            MessageContentPart::Image { media_type, path } => (media_type.as_str(), path.as_str()),
            _ => continue,
        };
        if !llm::is_normalizable_image(media_type) {
            continue;
        }
        let bytes = match std::fs::read(path) {
            Ok(bytes) => bytes,
            Err(_) => continue,
        };
        let name = Path::new(path)
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or(path);
        match llm::normalize_image_bytes(&bytes, media_type) {
            Ok((_, report)) if report.resized => eprintln!(
                "cortex run: image {name}: normalized {}x{} -> {}x{} ({}, {} -> {} bytes)",
                report.original_width,
                report.original_height,
                report.width,
                report.height,
                report.media_type,
                report.original_bytes,
                report.bytes,
            ),
            Ok((_, report)) => eprintln!(
                "cortex run: image {name}: {}x{} within upload bounds ({}, {} bytes)",
                report.width, report.height, report.media_type, report.bytes,
            ),
            Err(err) if bytes.len() > llm::MAX_IMAGE_BYTES => eprintln!(
                "cortex run: image {name}: rejected before upload ({} bytes, cap {}): {err}",
                bytes.len(),
                llm::MAX_IMAGE_BYTES,
            ),
            Err(_) => {}
        }
    }
}

fn extract_message_content_parts(parts: &[MessagePart]) -> Vec<MessageContentPart> {
    let mut seen = BTreeSet::new();
    let mut media = Vec::new();
    for part in parts {
        match &part.body {
            MessagePartBody::File {
                path, media_type, ..
            } => {
                if let Some(media_part) = message_media_part_for_file(path, media_type.as_deref())
                    .or_else(|| {
                        media_type_for_path(Path::new(path))
                            .and_then(|kind| message_media_part_for_file(path, Some(kind)))
                    })
                {
                    push_unique_media_part(&mut media, &mut seen, media_part);
                }
            }
            MessagePartBody::Reasoning { text, metadata } => {
                if let Some(reasoning_part) = reasoning_content_part(text, metadata.as_ref()) {
                    push_unique_media_part(&mut media, &mut seen, reasoning_part);
                }
            }
            MessagePartBody::Tool { attachments, .. } => {
                for attachment in attachments {
                    let media_part = match attachment {
                        ToolResultPart::Image { media_type, path } => {
                            Some(MessageContentPart::Image {
                                media_type: media_type.clone(),
                                path: path.clone(),
                            })
                        }
                        ToolResultPart::Document { media_type, path } => {
                            Some(MessageContentPart::Document {
                                media_type: media_type.clone(),
                                path: path.clone(),
                            })
                        }
                        ToolResultPart::Text { .. } => None,
                    };
                    if let Some(media_part) = media_part {
                        push_unique_media_part(&mut media, &mut seen, media_part);
                    }
                }
            }
            _ => {}
        }
    }
    media
}

fn reasoning_content_part(
    text: &str,
    metadata: Option<&serde_json::Value>,
) -> Option<MessageContentPart> {
    let metadata = metadata?;
    let object = metadata.as_object()?;
    let provider = object.get("provider").and_then(serde_json::Value::as_str)?;
    let signature = object
        .get("signature")
        .and_then(serde_json::Value::as_str)
        .map(ToOwned::to_owned);
    Some(MessageContentPart::Reasoning {
        provider: provider.to_owned(),
        text: text.to_owned(),
        signature,
    })
}

fn message_media_part_for_file(path: &str, media_type: Option<&str>) -> Option<MessageContentPart> {
    let kind = media_type?;
    if kind.starts_with("image/") {
        return Some(MessageContentPart::Image {
            media_type: kind.to_owned(),
            path: path.to_owned(),
        });
    }
    if kind == "application/pdf" {
        return Some(MessageContentPart::Document {
            media_type: kind.to_owned(),
            path: path.to_owned(),
        });
    }
    Some(MessageContentPart::File {
        path: path.to_owned(),
        media_type: Some(kind.to_owned()),
    })
}

fn push_unique_media_part(
    media: &mut Vec<MessageContentPart>,
    seen: &mut BTreeSet<String>,
    part: MessageContentPart,
) {
    let key = match &part {
        MessageContentPart::Image { media_type, path } => {
            format!("image|{media_type}|{path}")
        }
        MessageContentPart::Document { media_type, path } => {
            format!("document|{media_type}|{path}")
        }
        MessageContentPart::File { path, media_type } => {
            format!("file|{}|{path}", media_type.as_deref().unwrap_or(""))
        }
        MessageContentPart::Reasoning {
            provider,
            text,
            signature,
        } => {
            format!(
                "reasoning|{provider}|{text}|{}",
                signature.as_deref().unwrap_or_default()
            )
        }
    };
    if seen.insert(key) {
        media.push(part);
    }
}

/// Persist the assistant's reply so a later run resuming the same session sees it. A persistence
/// failure is reported on stderr but does not change the exit code: the answer has already been
/// delivered to the user, so the run itself succeeded.
fn persist_assistant(
    db: &Database,
    session_id: &str,
    outcome: &RunOutcome,
    parts: &[MessagePart],
) -> Option<String> {
    match db
        .messages()
        .create_with_parts(session_id, "assistant", outcome.answer(), parts)
    {
        Ok(message) => Some(message.id),
        Err(err) => {
            eprintln!("cortex run: failed to persist the assistant reply: {err}");
            None
        }
    }
}

/// Finalize the turn recorder and convert its recorded snapshot into a rich assistant part.
///
/// The payload intentionally carries only file paths and change classes, never raw file content.
fn checkpoint_snapshot_part(
    turn_message_id: &str,
    recorder: Option<&TurnRecorder>,
) -> Option<MessagePart> {
    let turn = recorder.and_then(crate::checkpoint::finalize_turn)?;
    snapshot_part_from_turn(turn_message_id, &turn)
}

fn snapshot_part_from_turn(
    turn_message_id: &str,
    turn: &checkpoints::RecordedTurn,
) -> Option<MessagePart> {
    let mut created = Vec::new();
    let mut updated = Vec::new();
    let mut deleted = Vec::new();

    for file in &turn.files {
        match (&file.pre, &file.post) {
            (None, Some(_)) => created.push(file.rel_path.clone()),
            (Some(_), None) => deleted.push(file.rel_path.clone()),
            (Some(_), Some(_)) => updated.push(file.rel_path.clone()),
            (None, None) => {}
        }
    }

    if created.is_empty() && updated.is_empty() && deleted.is_empty() {
        return None;
    }

    created.sort();
    updated.sort();
    deleted.sort();

    Some(MessagePart {
        id: format!("part_{}_snapshot_1", turn_message_id),
        created_at: None,
        body: MessagePartBody::Snapshot {
            label: turn.label.clone(),
            created,
            updated,
            deleted,
        },
    })
}

/// Persist compaction summary metadata as system messages so replay/export surfaces include the
/// summary text and tail boundary that produced the continued answer.
fn persist_compactions(db: &Database, session_id: &str, outcome: &RunOutcome) {
    for compaction in &outcome.compactions {
        let legacy_content = serialize_legacy_compaction_summary_content(
            &compaction.summary,
            &compaction.tail_start_id,
            compaction.continued_request,
        );
        let part = MessagePart {
            id: String::new(),
            created_at: None,
            body: MessagePartBody::Compaction {
                summary: compaction.summary.clone(),
                tail_start_id: compaction.tail_start_id.clone(),
                continued_request: compaction.continued_request,
            },
        };
        if let Err(err) =
            db.messages()
                .create_with_parts(session_id, "system", &legacy_content, &[part])
        {
            eprintln!("cortex run: failed to persist compaction summary metadata: {err}");
            break;
        }
    }
}

#[derive(Debug, Serialize)]
struct MemoryProvenancePayload {
    r#type: &'static str,
    sources: Vec<MemoryProvenanceSource>,
}

#[derive(Debug, Serialize)]
struct MemoryProvenanceSource {
    kind: String,
    path: String,
}

fn persist_memory_provenance(db: &Database, session_id: &str, sources: &[memory::MemorySource]) {
    if sources.is_empty() {
        return;
    }
    let payload = MemoryProvenancePayload {
        r#type: "memory_provenance",
        sources: sources
            .iter()
            .map(|source| MemoryProvenanceSource {
                kind: source.kind.to_owned(),
                path: source.path.display().to_string(),
            })
            .collect(),
    };
    let serialized = match serde_json::to_string(&payload) {
        Ok(serialized) => serialized,
        Err(err) => {
            eprintln!("cortex run: failed to serialize memory provenance: {err}");
            return;
        }
    };
    if let Err(err) = db
        .messages()
        .create_with_parts(session_id, "system", &serialized, &[])
    {
        eprintln!("cortex run: failed to persist memory provenance: {err}");
    }
}

fn memory_provenance_from_content(content: &str) -> bool {
    let Ok(parsed) = serde_json::from_str::<serde_json::Value>(content) else {
        return false;
    };
    parsed
        .get("type")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|kind| kind == "memory_provenance")
}

fn legacy_compaction_summary_for_message(message: &db::Message) -> Option<String> {
    if !message.role.eq_ignore_ascii_case("system") {
        return None;
    }
    compaction_summary_from_parts(&message.parts)
        .or_else(|| legacy_compaction_summary_from_content(&message.content))
}

fn compaction_summary_from_parts(parts: &[MessagePart]) -> Option<String> {
    parts.iter().find_map(|part| match &part.body {
        MessagePartBody::Compaction { summary, .. } => Some(summary.clone()),
        _ => None,
    })
}

fn legacy_compaction_summary_from_content(content: &str) -> Option<String> {
    let parsed: serde_json::Value = serde_json::from_str(content).ok()?;
    if parsed.get("type")?.as_str()? != "compaction_summary" {
        return None;
    }
    // Restrict compatibility decoding to the known persisted compaction JSON shape.
    parsed.get("tail_start_id")?.as_str()?;
    parsed.get("continued_request")?.as_bool()?;
    parsed.get("summary")?.as_str().map(str::to_owned)
}

fn serialize_legacy_compaction_summary_content(
    summary: &str,
    tail_start_id: &str,
    continued_request: bool,
) -> String {
    serde_json::json!({
        "type": "compaction_summary",
        "summary": summary,
        "tail_start_id": tail_start_id,
        "continued_request": continued_request,
    })
    .to_string()
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

fn resolve_plugin_provider_auth(
    plugin_state: &PluginState,
    provider_config: &providers::ProviderConfigSet,
    provider_id: &str,
    audit: &audit::AuditSink,
) -> Option<serde_json::Value> {
    let inputs = provider_config.credential_inputs(provider_id);
    let credential = providers::resolve_credential(provider_id, &inputs).ok()?;
    // Register the resolved plugin credential (and any nested secret-bearing fields) with the audit
    // sink so a plugin that surfaces it through any audited record gets it scrubbed, not logged raw.
    register_plugin_credential_secrets(audit, credential.expose());
    let parsed = parse_plugin_auth_payload(credential.expose());
    plugin_state.load_auth_for_provider(provider_id, Some(parsed))
}

/// Register a plugin credential string and its nested secret-bearing values with the audit sink.
fn register_plugin_credential_secrets(audit: &audit::AuditSink, raw: &str) {
    let raw = raw.trim();
    if raw.is_empty() {
        return;
    }
    audit.register_secret(raw);
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(raw) {
        register_secret_credential_fields(audit, &value);
    }
}

/// Recursively register values held under secret-bearing keys (`key`, `token`, `access`, ...).
fn register_secret_credential_fields(audit: &audit::AuditSink, value: &serde_json::Value) {
    const SECRET_KEYS: &[&str] = &[
        "key", "token", "access", "refresh", "secret", "api_key", "password",
    ];
    match value {
        serde_json::Value::Object(map) => {
            for (key, inner) in map {
                if SECRET_KEYS.contains(&key.as_str())
                    && let Some(text) = inner.as_str()
                {
                    audit.register_secret(text);
                }
                register_secret_credential_fields(audit, inner);
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                register_secret_credential_fields(audit, item);
            }
        }
        _ => {}
    }
}

fn parse_plugin_auth_payload(raw: &str) -> serde_json::Value {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return serde_json::Value::Null;
    }
    serde_json::from_str::<serde_json::Value>(trimmed)
        .unwrap_or_else(|_| serde_json::json!({ "type": "api", "key": trimmed }))
}

fn resolve_small_model_for_turn(
    plugin_state: &PluginState,
    active_model: &str,
    configured_small_model: Option<&str>,
) -> Option<String> {
    let configured = configured_small_model
        .map(str::trim)
        .filter(|model| !model.is_empty())
        .unwrap_or(active_model);
    let selected = plugin_state.small_model_for(active_model, configured);
    let selected = selected.trim();
    if selected.is_empty() || selected == active_model {
        return None;
    }
    if let Err(message) = validate_model_id(selected) {
        eprintln!("cortex run: ignoring invalid small model `{selected}`: {message}");
        return None;
    }
    Some(selected.to_owned())
}

/// The result of awaiting a turn that may be interrupted by SIGINT.
enum TurnResult {
    /// The turn ran to completion (success or a normalized error in the outcome).
    Completed(Box<RunOutcome>),
    /// SIGINT (Ctrl-C) cancelled the turn at a safe await boundary before it completed.
    Interrupted,
}

pub(crate) struct AuditedProvider {
    inner: Arc<dyn Provider>,
    audit: Arc<audit::AuditSink>,
    provider: String,
    endpoint: String,
    host: String,
}

impl AuditedProvider {
    pub(crate) fn new(
        inner: Arc<dyn Provider>,
        audit: Arc<audit::AuditSink>,
        provider: impl Into<String>,
        endpoint: impl Into<String>,
    ) -> Self {
        let endpoint = endpoint.into();
        let host = audit::host_from_endpoint(&endpoint).unwrap_or_default();
        Self {
            inner,
            audit,
            provider: provider.into(),
            endpoint,
            host,
        }
    }
}

impl Provider for AuditedProvider {
    fn stream<'a>(&'a self, request: LlmRequest) -> BoxFuture<'a, Result<LlmStream, LlmError>> {
        Box::pin(async move {
            let model = request.model.clone();
            let extra_headers = request.extra_headers.clone();
            let started = Instant::now();
            let result = self.inner.stream(request).await;
            let outcome = if result.is_ok() { "success" } else { "error" };
            self.audit.record_provider_request(
                &self.provider,
                &model,
                "POST",
                &self.host,
                &self.endpoint,
                None,
                started.elapsed().as_millis(),
                outcome,
                &extra_headers,
            );
            result
        })
    }
}

/// A process-wide lifecycle signal shared by MCP setup and the model turn.
///
/// The MCP manager can spawn stdio children before a turn begins, so cleanup must observe signals
/// during that setup phase as well as while a provider stream is active.
struct LifecycleSignal {
    cancelled: AtomicBool,
    notify: tokio::sync::Notify,
}

impl LifecycleSignal {
    const fn new() -> Self {
        Self {
            cancelled: AtomicBool::new(false),
            notify: tokio::sync::Notify::const_new(),
        }
    }

    fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }

    async fn wait(&self) {
        while !self.is_cancelled() {
            self.notify.notified().await;
        }
    }

    async fn watch(&self) {
        #[cfg(unix)]
        {
            use tokio::signal::unix::{SignalKind, signal};

            match (
                signal(SignalKind::interrupt()),
                signal(SignalKind::terminate()),
            ) {
                (Ok(mut interrupt), Ok(mut terminate)) => {
                    tokio::select! {
                        _ = interrupt.recv() => {}
                        _ = terminate.recv() => {}
                    }
                }
                (Ok(mut interrupt), Err(_)) => {
                    let _ = interrupt.recv().await;
                }
                (Err(_), Ok(mut terminate)) => {
                    let _ = terminate.recv().await;
                }
                (Err(_), Err(_)) => {
                    let _ = tokio::signal::ctrl_c().await;
                }
            }
        }
        #[cfg(not(unix))]
        {
            let _ = tokio::signal::ctrl_c().await;
        }
        self.cancelled.store(true, Ordering::SeqCst);
        self.notify.notify_waiters();
    }
}

/// Run a turn against the provider, racing it against a lifecycle signal so the process cancels
/// cleanly at a safe boundary instead of being hard-killed mid-write.
///
/// On interrupt the in-flight runner future is dropped, which drops the upstream SSE stream and
/// aborts the connection (no orphaned children, no panic), and [`TurnResult::Interrupted`] is
/// returned so the caller can finalize output cleanly (VAL-RUN-005 / VAL-RUN-013).
async fn run_turn<F>(
    provider: &dyn llm::Provider,
    turn: Turn,
    dispatcher: &dyn ToolDispatcher,
    lifecycle_signal: &LifecycleSignal,
    sink: F,
) -> TurnResult
where
    F: FnMut(RunnerEvent),
{
    let runner = SessionRunner::default();
    tokio::select! {
        outcome = runner.run_with_tools(provider, turn, dispatcher, sink) => {
            TurnResult::Completed(Box::new(outcome))
        }
        () = lifecycle_signal.wait() => TurnResult::Interrupted,
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
#[allow(clippy::too_many_arguments)]
async fn stream_text(
    provider: &dyn llm::Provider,
    resolved_model: &ResolvedModel,
    model_id: &str,
    agent: &str,
    _mode: Option<&str>,
    session_id: &str,
    turn_message_id: &str,
    db: &Database,
    turn: Turn,
    dispatcher: &dyn ToolDispatcher,
    checkpoint_recorder: Option<&Arc<TurnRecorder>>,
    plugin_state: &PluginState,
    bus: Arc<EventBus>,
    lifecycle_signal: &LifecycleSignal,
) -> Exit {
    let stdout = io::stdout();
    let mut handle = stdout.lock();
    let mut wrote_any = false;
    let mut runner_events: Vec<RunnerEvent> = Vec::new();
    let mut lifecycle = PartLifecycleTracker::new(turn_message_id, model_id);

    let result = run_turn(provider, turn, dispatcher, lifecycle_signal, |event| {
        publish_runner_event(&bus, &event);
        let _ = lifecycle.observe(&event);
        if let RunnerEvent::TextDelta { text } = &event {
            if !text.is_empty() {
                let text = redaction::redact(text);
                let _ = handle.write_all(text.as_bytes());
                let _ = handle.flush();
                wrote_any = true;
            }
        }
        runner_events.push(event);
    })
    .await;
    let snapshot_part =
        checkpoint_snapshot_part(turn_message_id, checkpoint_recorder.map(Arc::as_ref));

    let mut outcome = match result {
        TurnResult::Completed(outcome) => *outcome,
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
    plugin_state.apply_compaction_hooks(model_id, &mut outcome.compactions);
    apply_estimated_usage_cost(resolved_model, &mut outcome);

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
    let part_timestamps = lifecycle.part_timestamps();
    let mut assistant_parts = build_assistant_message_parts(
        model_id,
        agent,
        turn_message_id,
        &runner_events,
        &outcome,
        &part_timestamps,
    );
    if let Some(snapshot_part) = snapshot_part {
        assistant_parts.push(snapshot_part);
    }
    persist_compactions(db, session_id, &outcome);
    if let Some(message_id) = persist_assistant(db, session_id, &outcome, &assistant_parts) {
        bus.publish(&Event::message_parts(
            session_id.to_owned(),
            message_id,
            assistant_parts.clone(),
        ));
    }
    persist_usage(db, session_id, &outcome);
    Exit::Success
}

/// Stream the runner's normalized event vocabulary, one JSON object per line. A leading `session`
/// event announces the session id so a machine consumer can capture and reuse it via `--session`.
#[allow(clippy::too_many_arguments)]
async fn stream_jsonl(
    provider: &dyn llm::Provider,
    resolved_model: &ResolvedModel,
    model_id: &str,
    agent: &str,
    mode: Option<&str>,
    session_id: &str,
    turn_message_id: &str,
    tool_names: &[String],
    tool_definitions: &[ToolDef],
    db: &Database,
    turn: Turn,
    dispatcher: &dyn ToolDispatcher,
    checkpoint_recorder: Option<&Arc<TurnRecorder>>,
    plugin_state: &PluginState,
    bus: Arc<EventBus>,
    lifecycle_signal: &LifecycleSignal,
) -> Exit {
    let stdout = io::stdout();
    let mut handle = stdout.lock();
    let mut runner_events: Vec<RunnerEvent> = Vec::new();
    let mut lifecycle = PartLifecycleTracker::new(turn_message_id, model_id);
    // On a TTY each event line is syntax-highlighted and a footer is appended; off a TTY the
    // styler is a pure passthrough so the machine bytes stay byte-identical, footer suppressed
    // (VAL-CLI-008/010/011).
    let styler = style::Styler::for_stdout();
    let mut event_count: usize = 0;

    if let Ok(line) = render_session_line(agent, mode, session_id, tool_names, tool_definitions) {
        write_jsonl_line(&mut handle, styler, &line, &mut event_count);
    }

    let result = run_turn(provider, turn, dispatcher, lifecycle_signal, |event| {
        publish_runner_event(&bus, &event);
        for part_event in lifecycle.observe(&event) {
            if let Ok(line) = render_part_lifecycle_line(&part_event) {
                write_jsonl_line(&mut handle, styler, &line, &mut event_count);
            }
        }
        if let Ok(line) = render_event_line(&event) {
            write_jsonl_line(&mut handle, styler, &line, &mut event_count);
        }
        runner_events.push(event);
    })
    .await;
    let snapshot_part =
        checkpoint_snapshot_part(turn_message_id, checkpoint_recorder.map(Arc::as_ref));

    let mut outcome = match result {
        TurnResult::Completed(outcome) => *outcome,
        TurnResult::Interrupted => {
            // Every JSONL line already written is a complete object; stop emitting and exit. The
            // session stays durable (the prompt was persisted before the turn).
            drop(handle);
            eprintln!("cortex run: interrupted");
            return Exit::Interrupted;
        }
    };
    plugin_state.apply_compaction_hooks(model_id, &mut outcome.compactions);
    apply_estimated_usage_cost(resolved_model, &mut outcome);

    // TTY-only stream footer: a faint `stream complete · N events` summary. Suppressed off a TTY
    // (the styler is disabled) so piped JSONL stays byte-identical to the baseline (VAL-CLI-010/011).
    if styler.is_enabled() {
        let footer = format!("stream complete {} {event_count} events", style::SEPARATOR);
        let _ = handle.write_all(styler.paint(&footer, style::FAINT).as_bytes());
        let _ = handle.write_all(b"\n");
        let _ = handle.flush();
    }

    drop(handle);
    if let Some(err) = &outcome.error {
        eprintln!("cortex run: {err}");
        return err.exit();
    }
    let part_timestamps = lifecycle.part_timestamps();
    let mut assistant_parts = build_assistant_message_parts(
        model_id,
        agent,
        turn_message_id,
        &runner_events,
        &outcome,
        &part_timestamps,
    );
    if let Some(snapshot_part) = snapshot_part {
        assistant_parts.push(snapshot_part);
    }
    persist_compactions(db, session_id, &outcome);
    if let Some(message_id) = persist_assistant(db, session_id, &outcome, &assistant_parts) {
        bus.publish(&Event::message_parts(
            session_id.to_owned(),
            message_id,
            assistant_parts.clone(),
        ));
    }
    persist_usage(db, session_id, &outcome);
    Exit::Success
}

/// A single JSON document describing a completed turn for `--format json`.
#[derive(Serialize)]
struct RunDocument<'a> {
    role: &'a str,
    content: &'a str,
    /// Parsed assistant output validated against `--schema`, when requested.
    #[serde(skip_serializing_if = "Option::is_none")]
    structured_output: Option<serde_json::Value>,
    model: &'a str,
    /// The agent persona this turn ran under (the value passed to `--agent`, or the default).
    agent: &'a str,
    /// Optional mode overlay composed with the selected agent.
    #[serde(skip_serializing_if = "Option::is_none")]
    mode: Option<&'a str>,
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
    /// The full tool definitions advertised this turn, including descriptions and JSON schema.
    /// This lets validators inspect dynamic tool-specific availability such as the `task`
    /// subagent target enum (VAL-AGENT-013).
    tool_definitions: &'a [ToolDef],
    /// The structured results of every tool the model invoked this turn, in call order. Carries
    /// text parts and any media attachments (images/PDFs) so a machine consumer sees the tool
    /// output without binary bytes leaking into the text channel. Omitted when no tool was called.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tool_results: Vec<ToolResultDoc>,
    /// Workspace adapters registered by plugins through
    /// `experimental_workspace.register(type, adapter)`.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    workspace_adapters: Vec<WorkspaceAdapterDoc>,
    /// Rich persisted message parts for this assistant turn.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    parts: Vec<MessagePart>,
    /// Compaction metadata captured during this turn (summary text + preserved-tail boundary).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    compactions: Vec<CompactionDoc>,
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

/// One compaction summary captured during a turn.
#[derive(Serialize)]
struct CompactionDoc {
    summary: String,
    tail_start_id: String,
    continued_request: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    model: Option<String>,
}

/// The leading `--jsonl` event announcing the agent persona and session id, before the runner's
/// own events.
#[derive(Serialize)]
struct SessionEvent<'a> {
    #[serde(rename = "type")]
    kind: &'a str,
    /// The agent persona this turn ran under (the value passed to `--agent`, or the default).
    agent: &'a str,
    /// Optional mode overlay composed with the selected agent.
    #[serde(skip_serializing_if = "Option::is_none")]
    mode: Option<&'a str>,
    /// The stable, re-usable session id this turn ran under (feed back via `--session`).
    session: &'a str,
    /// The per-turn tool set materialized for the model's family, in advertised order (the
    /// observable per-turn tool advertisement; see [`RunDocument::tools`]).
    tools: &'a [String],
    /// Full advertised tool definitions for this turn.
    tool_definitions: &'a [ToolDef],
}

/// Run the turn to completion, then emit a single strict JSON document on stdout. No partial
/// document is written if the turn fails.
#[allow(clippy::too_many_arguments)]
async fn emit_json(
    provider: &dyn llm::Provider,
    resolved_model: &ResolvedModel,
    model_id: &str,
    agent: &str,
    mode: Option<&str>,
    session_id: &str,
    turn_message_id: &str,
    db: &Database,
    turn: Turn,
    dispatcher: &dyn ToolDispatcher,
    checkpoint_recorder: Option<&Arc<TurnRecorder>>,
    plugin_state: &PluginState,
    bus: Arc<EventBus>,
    lifecycle_signal: &LifecycleSignal,
    schema: Option<&serde_json::Value>,
    workspace_adapters: &[WorkspaceAdapterDoc],
) -> Exit {
    // Collect each tool result as it streams so the completed document can carry the structured
    // tool output (text plus any media attachments) alongside the assistant answer.
    let mut tool_results: Vec<ToolResultDoc> = Vec::new();
    let mut runner_events: Vec<RunnerEvent> = Vec::new();
    let mut lifecycle = PartLifecycleTracker::new(turn_message_id, model_id);
    let result = run_turn(provider, turn, dispatcher, lifecycle_signal, |event| {
        publish_runner_event(&bus, &event);
        let _ = lifecycle.observe(&event);
        if let RunnerEvent::ToolResult {
            id,
            name,
            parts,
            is_error,
        } = &event
        {
            tool_results.push(ToolResultDoc {
                id: id.clone(),
                name: name.clone(),
                parts: parts.clone(),
                is_error: *is_error,
            });
        }
        runner_events.push(event);
    })
    .await;
    let snapshot_part =
        checkpoint_snapshot_part(turn_message_id, checkpoint_recorder.map(Arc::as_ref));

    let mut outcome = match result {
        TurnResult::Completed(outcome) => *outcome,
        TurnResult::Interrupted => {
            // The single JSON document is written only after the turn completes, so an interrupt
            // here leaves stdout byte-empty — never a partial/malformed document (VAL-RUN-013).
            eprintln!("cortex run: interrupted");
            return Exit::Interrupted;
        }
    };
    plugin_state.apply_compaction_hooks(model_id, &mut outcome.compactions);
    apply_estimated_usage_cost(resolved_model, &mut outcome);

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
    let final_tool_definitions = dispatcher.definitions();
    let final_tool_names = tool_names_from(&final_tool_definitions);
    let part_timestamps = lifecycle.part_timestamps();
    let mut assistant_parts = build_assistant_message_parts(
        model_id,
        agent,
        turn_message_id,
        &runner_events,
        &outcome,
        &part_timestamps,
    );
    if let Some(snapshot_part) = snapshot_part {
        assistant_parts.push(snapshot_part);
    }
    let structured_output = match validate_structured_output(outcome.answer(), schema) {
        Ok(value) => value,
        Err(diag) => {
            eprintln!("cortex run: {}: {diag}", diag.kind());
            return diag.exit();
        }
    };

    let line = match render_json_document(
        model_id,
        agent,
        mode,
        session_id,
        &final_tool_names,
        &final_tool_definitions,
        &outcome,
        session_usage,
        tool_results,
        workspace_adapters.to_vec(),
        assistant_parts.clone(),
        structured_output,
    ) {
        Ok(line) => line,
        Err(err) => {
            eprintln!("cortex run: failed to serialize output: {err}");
            return Exit::Failure;
        }
    };

    let stdout = io::stdout();
    let mut handle = stdout.lock();
    // On a TTY the human branch pretty-prints + syntax-highlights the document; off a TTY the
    // styler is a pure passthrough so the machine bytes stay byte-identical (VAL-CLI-004/007).
    let styler = style::Styler::for_stdout();
    let rendered = if styler.is_enabled() {
        format!(
            "{}\n",
            styler.highlight_json_pretty(line.trim_end_matches('\n'))
        )
    } else {
        line
    };
    if handle
        .write_all(rendered.as_bytes())
        .and_then(|()| handle.flush())
        .is_err()
    {
        return Exit::Failure;
    }
    drop(handle);
    persist_compactions(db, session_id, &outcome);
    if let Some(message_id) = persist_assistant(db, session_id, &outcome, &assistant_parts) {
        bus.publish(&Event::message_parts(
            session_id.to_owned(),
            message_id,
            assistant_parts.clone(),
        ));
    }
    Exit::Success
}

/// Build the single JSON document describing a completed turn (role, content, model, session id,
/// finish reason, and the optional usage object).
#[allow(clippy::too_many_arguments)]
fn build_run_document<'a>(
    model_id: &'a str,
    agent: &'a str,
    mode: Option<&'a str>,
    session_id: &'a str,
    tool_names: &'a [String],
    tool_definitions: &'a [ToolDef],
    outcome: &'a RunOutcome,
    session_usage: Option<UsageTotals>,
    tool_results: Vec<ToolResultDoc>,
    workspace_adapters: Vec<WorkspaceAdapterDoc>,
    parts: Vec<MessagePart>,
    structured_output: Option<serde_json::Value>,
) -> RunDocument<'a> {
    RunDocument {
        role: "assistant",
        content: outcome.answer(),
        structured_output,
        model: model_id,
        agent,
        mode,
        session: session_id,
        finish_reason: &outcome.finish_reason,
        steps: outcome.steps,
        usage: outcome.usage.as_ref(),
        session_usage,
        tools: tool_names,
        tool_definitions,
        tool_results,
        workspace_adapters,
        parts,
        compactions: outcome
            .compactions
            .iter()
            .map(|record| CompactionDoc {
                summary: record.summary.clone(),
                tail_start_id: record.tail_start_id.clone(),
                continued_request: record.continued_request,
                model: record.model.clone(),
            })
            .collect(),
    }
}

/// Serialize the completed-turn document to a single line terminated by exactly one newline, so
/// stdout carries one strict JSON value with no leading or trailing non-JSON bytes.
#[allow(clippy::too_many_arguments)]
fn render_json_document(
    model_id: &str,
    agent: &str,
    mode: Option<&str>,
    session_id: &str,
    tool_names: &[String],
    tool_definitions: &[ToolDef],
    outcome: &RunOutcome,
    session_usage: Option<UsageTotals>,
    tool_results: Vec<ToolResultDoc>,
    workspace_adapters: Vec<WorkspaceAdapterDoc>,
    parts: Vec<MessagePart>,
    structured_output: Option<serde_json::Value>,
) -> Result<String, serde_json::Error> {
    let mut line = serde_json::to_string(&build_run_document(
        model_id,
        agent,
        mode,
        session_id,
        tool_names,
        tool_definitions,
        outcome,
        session_usage,
        tool_results,
        workspace_adapters,
        parts,
        structured_output,
    ))?;
    line.push('\n');
    Ok(redaction::redact(&line))
}

/// Render the leading `--jsonl` session event (without the trailing newline). It announces the
/// agent persona, the session id, and the per-turn tool advertisement.
fn render_session_line(
    agent: &str,
    mode: Option<&str>,
    session_id: &str,
    tool_names: &[String],
    tool_definitions: &[ToolDef],
) -> Result<String, serde_json::Error> {
    serde_json::to_string(&SessionEvent {
        kind: "session",
        agent,
        mode,
        session: session_id,
        tools: tool_names,
        tool_definitions,
    })
    .map(|line| redaction::redact(&line))
}

/// Project a full tool-definition snapshot to its advertised names, preserving order.
fn tool_names_from(tool_definitions: &[ToolDef]) -> Vec<String> {
    tool_definitions
        .iter()
        .map(|definition| definition.name.clone())
        .collect()
}

/// Render one normalized runner event as a single JSONL line (without the trailing newline). Every
/// event serializes to an object carrying a stable `type` discriminator.
fn render_event_line(event: &RunnerEvent) -> Result<String, serde_json::Error> {
    serde_json::to_string(event).map(|line| redaction::redact(&line))
}

/// Write one JSONL event line, syntax-highlighting it when the styler is enabled (TTY), and bump
/// the emitted-event counter. Off a TTY the styler is a pure passthrough, so the raw machine bytes
/// (`line` + `\n`) reach a piped consumer unchanged.
fn write_jsonl_line<W: Write>(
    handle: &mut W,
    styler: style::Styler,
    line: &str,
    event_count: &mut usize,
) {
    let rendered = styler.highlight_json(line);
    let _ = handle.write_all(rendered.as_bytes());
    let _ = handle.write_all(b"\n");
    let _ = handle.flush();
    *event_count += 1;
}

/// Render one derived part lifecycle event as a JSONL line.
fn render_part_lifecycle_line(event: &PartLifecycleEvent) -> Result<String, serde_json::Error> {
    serde_json::to_string(event).map(|line| redaction::redact(&line))
}

/// Publish runner tool events onto the typed event bus so plugin `event` hooks and future audit
/// sinks observe the same concrete lifecycle as machine-output consumers.
fn publish_runner_event(bus: &EventBus, event: &RunnerEvent) {
    match event {
        RunnerEvent::ToolCall {
            id,
            name,
            arguments,
        } => bus.publish(&Event::tool_call(
            id.clone(),
            name.clone(),
            arguments.clone(),
        )),
        RunnerEvent::ToolResult {
            id, name, is_error, ..
        } => bus.publish(&Event::tool_result(id.clone(), name.clone(), *is_error)),
        RunnerEvent::Retry { .. } => {}
        _ => {}
    }
}

/// Collapse the two output-selection inputs into a single effective format. The `--jsonl`
/// shorthand is mutually exclusive with `--format` at the arg layer, so it wins only when set.
fn effective_format(jsonl: bool, format: OutputFormat) -> OutputFormat {
    if jsonl { OutputFormat::Jsonl } else { format }
}

fn parse_structured_schema(
    raw: Option<&str>,
    format: OutputFormat,
) -> Result<Option<serde_json::Value>, String> {
    let Some(raw) = raw else {
        return Ok(None);
    };
    if format != OutputFormat::Json {
        return Err("--schema is supported only with --format json".to_owned());
    }
    serde_json::from_str::<serde_json::Value>(raw)
        .map(Some)
        .map_err(|err| format!("invalid --schema JSON: {err}"))
}

fn validate_structured_output(
    answer: &str,
    schema: Option<&serde_json::Value>,
) -> Result<Option<serde_json::Value>, Diagnostic> {
    let Some(schema) = schema else {
        return Ok(None);
    };
    let parsed = serde_json::from_str::<serde_json::Value>(answer.trim()).map_err(|err| {
        Diagnostic::schema_validation(format!("assistant output is not valid JSON: {err}"))
    })?;
    if let Err(detail) = validate_schema_value(schema, &parsed, "$") {
        return Err(Diagnostic::schema_validation(detail));
    }
    Ok(Some(parsed))
}

fn validate_schema_value(
    schema: &serde_json::Value,
    value: &serde_json::Value,
    path: &str,
) -> Result<(), String> {
    if let Some(allow) = schema.as_bool() {
        return if allow {
            Ok(())
        } else {
            Err(format!("{path}: schema forbids all values"))
        };
    }

    let Some(object) = schema.as_object() else {
        return Ok(());
    };

    if let Some(constraint) = object.get("const")
        && value != constraint
    {
        return Err(format!("{path}: value does not match const constraint"));
    }
    if let Some(options) = object.get("enum").and_then(|v| v.as_array())
        && !options.iter().any(|candidate| candidate == value)
    {
        return Err(format!("{path}: value is not in enum"));
    }

    if let Some(type_keyword) = object.get("type")
        && !schema_type_matches(type_keyword, value)
    {
        return Err(format!(
            "{path}: expected type {type_keyword}, got {}",
            json_type_name(value)
        ));
    }

    if let Some(all_of) = object.get("allOf").and_then(|v| v.as_array()) {
        for sub_schema in all_of {
            validate_schema_value(sub_schema, value, path)?;
        }
    }
    if let Some(any_of) = object.get("anyOf").and_then(|v| v.as_array()) {
        let mut matched = false;
        for sub_schema in any_of {
            if validate_schema_value(sub_schema, value, path).is_ok() {
                matched = true;
                break;
            }
        }
        if !matched {
            return Err(format!("{path}: value does not satisfy anyOf"));
        }
    }
    if let Some(one_of) = object.get("oneOf").and_then(|v| v.as_array()) {
        let matches = one_of
            .iter()
            .filter(|sub_schema| validate_schema_value(sub_schema, value, path).is_ok())
            .count();
        if matches != 1 {
            return Err(format!(
                "{path}: value must satisfy exactly one oneOf branch"
            ));
        }
    }

    if let Some(required) = object.get("required").and_then(|v| v.as_array()) {
        let Some(map) = value.as_object() else {
            return Err(format!(
                "{path}: required fields apply only to object values"
            ));
        };
        for key in required {
            let Some(field) = key.as_str() else {
                continue;
            };
            if !map.contains_key(field) {
                return Err(format!("{path}: missing required field `{field}`"));
            }
        }
    }

    if let Some(properties) = object.get("properties").and_then(|v| v.as_object())
        && let Some(map) = value.as_object()
    {
        for (key, sub_schema) in properties {
            if let Some(child) = map.get(key) {
                validate_schema_value(sub_schema, child, &json_path(path, key))?;
            }
        }
        if object.get("additionalProperties").and_then(|v| v.as_bool()) == Some(false) {
            for key in map.keys() {
                if !properties.contains_key(key) {
                    return Err(format!("{path}: unexpected property `{key}`"));
                }
            }
        }
    }

    if let Some(items) = object.get("items")
        && let Some(array) = value.as_array()
    {
        for (index, item) in array.iter().enumerate() {
            validate_schema_value(items, item, &format!("{path}[{index}]"))?;
        }
    }

    if let Some(min_items) = object
        .get("minItems")
        .and_then(serde_json::Value::as_u64)
        .map(|n| n as usize)
        && value
            .as_array()
            .is_some_and(|array| array.len() < min_items)
    {
        return Err(format!("{path}: expected at least {min_items} item(s)"));
    }
    if let Some(max_items) = object
        .get("maxItems")
        .and_then(serde_json::Value::as_u64)
        .map(|n| n as usize)
        && value
            .as_array()
            .is_some_and(|array| array.len() > max_items)
    {
        return Err(format!("{path}: expected at most {max_items} item(s)"));
    }

    if let Some(min_length) = object
        .get("minLength")
        .and_then(serde_json::Value::as_u64)
        .map(|n| n as usize)
        && value
            .as_str()
            .is_some_and(|text| text.chars().count() < min_length)
    {
        return Err(format!("{path}: expected string length >= {min_length}"));
    }
    if let Some(max_length) = object
        .get("maxLength")
        .and_then(serde_json::Value::as_u64)
        .map(|n| n as usize)
        && value
            .as_str()
            .is_some_and(|text| text.chars().count() > max_length)
    {
        return Err(format!("{path}: expected string length <= {max_length}"));
    }

    if let Some(minimum) = object.get("minimum").and_then(serde_json::Value::as_f64)
        && value.as_f64().is_some_and(|number| number < minimum)
    {
        return Err(format!("{path}: expected number >= {minimum}"));
    }
    if let Some(maximum) = object.get("maximum").and_then(serde_json::Value::as_f64)
        && value.as_f64().is_some_and(|number| number > maximum)
    {
        return Err(format!("{path}: expected number <= {maximum}"));
    }

    Ok(())
}

fn schema_type_matches(type_keyword: &serde_json::Value, value: &serde_json::Value) -> bool {
    if let Some(kind) = type_keyword.as_str() {
        return json_type_matches(kind, value);
    }
    type_keyword.as_array().is_some_and(|kinds| {
        kinds
            .iter()
            .filter_map(serde_json::Value::as_str)
            .any(|kind| json_type_matches(kind, value))
    })
}

fn json_type_matches(kind: &str, value: &serde_json::Value) -> bool {
    match kind {
        "null" => value.is_null(),
        "boolean" => value.is_boolean(),
        "object" => value.is_object(),
        "array" => value.is_array(),
        "number" => value.is_number(),
        "integer" => value.as_i64().is_some() || value.as_u64().is_some(),
        "string" => value.is_string(),
        _ => false,
    }
}

fn json_type_name(value: &serde_json::Value) -> &'static str {
    if value.is_null() {
        "null"
    } else if value.is_boolean() {
        "boolean"
    } else if value.is_object() {
        "object"
    } else if value.is_array() {
        "array"
    } else if value.as_i64().is_some() || value.as_u64().is_some() {
        "integer"
    } else if value.is_number() {
        "number"
    } else if value.is_string() {
        "string"
    } else {
        "unknown"
    }
}

fn json_path(parent: &str, key: &str) -> String {
    if parent == "$" {
        format!("$.{key}")
    } else {
        format!("{parent}.{key}")
    }
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

/// The layered default agent for `cortex run` when no `--agent` flag is supplied.
///
/// # Errors
/// Returns the underlying [`ConfigError`] when a configuration layer cannot be loaded.
fn configured_agent_name(inputs: &ConfigInputs) -> Result<Option<String>, ConfigError> {
    let effective = settings::load(inputs.clone())?;
    Ok(non_empty(effective.agent()))
}

/// The layered default mode for `cortex run` when no `--mode` flag is supplied.
///
/// # Errors
/// Returns the underlying [`ConfigError`] when a configuration layer cannot be loaded.
fn configured_mode_name(inputs: &ConfigInputs) -> Result<Option<String>, ConfigError> {
    let effective = settings::load(inputs.clone())?;
    Ok(non_empty(
        effective
            .get("mode")
            .and_then(toml::Value::as_str)
            .unwrap_or_default(),
    ))
}

/// The configuration-derived inputs for a run: the resolved model, typed provider policy/config,
/// and additional local runtime integrations.
struct RunConfig {
    model: ResolvedModel,
    provider_config: providers::ProviderConfigSet,
    mcp_servers: std::collections::BTreeMap<String, mcp::McpServerConfig>,
    formatter_config: format::FormatterConfig,
    lsp_config: lsp::LspConfigSet,
    plugins: plugins::PluginConfig,
    websearch: Option<tools::WebSearchConfig>,
    resolved_config: serde_json::Value,
}

/// Resolve everything a run derives from configuration: merge every layer (the `--model` flag is
/// carried as the CLI override inside `inputs`), resolve the winning model id against the provider
/// catalogue, and parse typed provider configuration and policy.
///
/// # Errors
/// Returns the underlying [`ConfigError`] when a configuration layer cannot be loaded (bad TOML,
/// failed `{env:}`/`{file:}` substitution, or a type mismatch).
fn resolve_run_config(inputs: ConfigInputs) -> Result<RunConfig, ConfigError> {
    let cwd = inputs.cwd.clone();
    let effective = settings::load(inputs)?;
    let provider_config = provider_config_set(&effective)?;
    let formatter_config =
        format::FormatterConfig::from_table(effective.table()).map_err(|message| {
            ConfigError::TypeMismatch {
                key: "formatter".to_owned(),
                expected: "a formatter config table",
                found: message,
            }
        })?;
    let plugins = plugins::PluginConfig::from_toml_value(effective.get("plugin"), &cwd).map_err(
        |message| ConfigError::TypeMismatch {
            key: "plugin".to_owned(),
            expected: "an array of plugin specs or [spec, options] pairs",
            found: message,
        },
    )?;
    let websearch = tools::WebSearchConfig::from_effective(&effective)?;
    let model = provider_config
        .resolve_model(providers::Catalogue::embedded(), Some(effective.model()))
        .map_err(provider_config_error)?;
    Ok(RunConfig {
        model,
        provider_config,
        mcp_servers: mcp_servers_from_config(&effective)?,
        formatter_config,
        lsp_config: crate::lsp_config::from_effective(&effective)?,
        plugins,
        websearch,
        resolved_config: serde_json::to_value(effective.table())
            .unwrap_or_else(|_| serde_json::Value::Object(Default::default())),
    })
}

/// Extract the configured MCP servers from the effective config's `mcp` table.
pub(crate) fn mcp_servers_from_config(
    effective: &EffectiveConfig,
) -> Result<std::collections::BTreeMap<String, mcp::McpServerConfig>, ConfigError> {
    let Some(value) = effective.get("mcp") else {
        return Ok(std::collections::BTreeMap::new());
    };
    let json = serde_json::to_value(value).map_err(|err| ConfigError::TypeMismatch {
        key: "mcp".to_owned(),
        expected: "a valid MCP server configuration",
        found: err.to_string(),
    })?;
    mcp::servers_from_value(&json).map_err(|err| ConfigError::TypeMismatch {
        key: "mcp".to_owned(),
        expected: "a valid MCP server configuration",
        found: err.to_string(),
    })
}

/// Resolve the effective model for the turn (a thin wrapper over [`resolve_run_config`]).
///
/// # Errors
/// Returns the underlying [`ConfigError`] when a configuration layer cannot be loaded.
#[cfg(test)]
fn resolve_run_model(inputs: ConfigInputs) -> Result<ResolvedModel, ConfigError> {
    Ok(resolve_run_config(inputs)?.model)
}

fn provider_config_set(
    effective: &EffectiveConfig,
) -> Result<providers::ProviderConfigSet, ConfigError> {
    providers::ProviderConfigSet::from_table(effective.table()).map_err(provider_config_error)
}

fn provider_config_error(error: providers::ProviderConfigError) -> ConfigError {
    ConfigError::TypeMismatch {
        key: error.key,
        expected: "a valid provider configuration",
        found: error.message,
    }
}

/// Trim a candidate value and keep it only if non-empty after trimming.
fn non_empty(value: &str) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_owned())
}

/// Build the compaction policy for the run from the resolved model's context window and per-turn
/// output limit. The effective window can be overridden via `CORTEX_CONTEXT_LIMIT_TOKENS` (an
/// operational tuning/testing knob); an unknown model falls back to the engine's default window.
pub(crate) fn compaction_policy(model: &ResolvedModel) -> CompactionPolicy {
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
pub(crate) fn validate_model_id(model_id: &str) -> Result<(), String> {
    providers::validate_model_id(model_id)
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
    use engine::Agent;
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
        let discovery = custom_agents::Discovery::default();
        assert_eq!(
            custom_agents::resolve(None, &discovery),
            Ok(custom_agents::ActiveAgent::Builtin(Agent::default()))
        );
        assert_eq!(
            custom_agents::resolve(Some("plan"), &discovery),
            Ok(custom_agents::ActiveAgent::Builtin(Agent::Plan))
        );
        assert_eq!(
            custom_agents::resolve(Some("build"), &discovery),
            Ok(custom_agents::ActiveAgent::Builtin(Agent::Build))
        );
        assert_eq!(
            custom_agents::resolve(Some(" General "), &discovery),
            Ok(custom_agents::ActiveAgent::Builtin(Agent::General))
        );
        assert_eq!(
            custom_agents::resolve(Some("EXPLORE"), &discovery),
            Ok(custom_agents::ActiveAgent::Builtin(Agent::Explore))
        );
    }

    // VAL-LLM-028 (resolution half): an unknown agent name is rejected with an error that names the
    // offending agent and hints the available agents, so the CLI can fail fast before any network.
    #[test]
    fn resolve_agent_rejects_unknown_names_with_an_actionable_error() {
        let discovery = custom_agents::Discovery::default();
        let err = custom_agents::resolve(Some("no-such-agent"), &discovery)
            .expect_err("unknown agent must be rejected");
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
            run_config
                .provider_config
                .provider_base_url(providers::OPENROUTER_ID)
                .as_deref(),
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
            run_config
                .provider_config
                .provider_base_url(providers::OPENROUTER_ID)
                .as_deref(),
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
            run_config
                .provider_config
                .provider_base_url(providers::OPENROUTER_ID)
                .as_deref(),
            Some("http://127.0.0.1:3299")
        );
    }

    // With nothing configured the override is absent, so the catalogue default base URL is used.
    #[test]
    fn no_base_url_override_by_default() {
        let (base, proj) = scratch_tree("base-url-none");
        let run_config =
            resolve_run_config(config_inputs(&base, &proj, &[], None)).expect("resolve");
        assert!(
            run_config
                .provider_config
                .provider_base_url(providers::OPENROUTER_ID)
                .is_none()
        );
        assert!(
            run_config
                .provider_config
                .provider(providers::OPENROUTER_ID)
                .and_then(|provider| provider.auth.api_key.clone())
                .is_none()
        );
        assert!(
            run_config.websearch.is_none(),
            "websearch must be default-off with no provider enabled"
        );
    }

    #[test]
    fn websearch_config_enables_duckduckgo_provider() {
        let (base, proj) = scratch_tree("websearch-duckduckgo");
        write_config(
            &proj,
            "[websearch]\nenabled = true\nprovider = \"duckduckgo\"\nmax_results = 4\n",
        );
        let run_config =
            resolve_run_config(config_inputs(&base, &proj, &[], None)).expect("resolve");
        let websearch = run_config.websearch.expect("websearch config");
        assert_eq!(websearch.provider(), tools::WebSearchProvider::DuckDuckGo);
        assert_eq!(websearch.max_results(), 4);
        assert!(!websearch.has_api_key());
    }

    #[test]
    fn websearch_flat_env_keys_override_nested_config() {
        let (base, proj) = scratch_tree("websearch-env");
        write_config(
            &proj,
            "[websearch]\nenabled = false\nprovider = \"duckduckgo\"\nmax_results = 2\n",
        );
        let run_config = resolve_run_config(config_inputs(
            &base,
            &proj,
            &[
                ("CORTEX_WEBSEARCH_ENABLED", "true"),
                ("CORTEX_WEBSEARCH_PROVIDER", "brave"),
                ("CORTEX_WEBSEARCH_API_KEY", "from-env"),
                ("CORTEX_WEBSEARCH_MAX_RESULTS", "3"),
            ],
            None,
        ))
        .expect("resolve");
        let websearch = run_config.websearch.expect("websearch config");
        assert_eq!(websearch.provider(), tools::WebSearchProvider::Brave);
        assert_eq!(websearch.max_results(), 3);
        assert!(websearch.has_api_key());
    }

    #[test]
    fn websearch_config_rejects_bad_provider() {
        let (base, proj) = scratch_tree("websearch-bad-provider");
        write_config(&proj, "[websearch]\nenabled = true\nprovider = \"bogus\"\n");
        let err = match resolve_run_config(config_inputs(&base, &proj, &[], None)) {
            Ok(_) => panic!("bad provider should fail"),
            Err(err) => err,
        };
        let rendered = err.to_string();
        assert!(rendered.contains("websearch.provider"), "{rendered}");
        assert!(rendered.contains("duckduckgo") && rendered.contains("brave"));
    }

    // A nested `[provider.openrouter] api_key` is surfaced as the config-layer credential (the
    // lowest-precedence credential source, below the env var and the 0600 store).
    #[test]
    fn api_key_from_nested_provider_table() {
        let (base, proj) = scratch_tree("api-key-nested");
        write_config(&proj, "[provider.openrouter]\napi_key = \"from-config\"\n");
        let run_config =
            resolve_run_config(config_inputs(&base, &proj, &[], None)).expect("resolve");
        assert_eq!(
            run_config
                .provider_config
                .provider(providers::OPENROUTER_ID)
                .and_then(|provider| provider.auth.api_key.as_deref()),
            Some("from-config")
        );
    }

    // A flat `api_key` key is also accepted as the config-layer credential.
    #[test]
    fn api_key_from_flat_key() {
        let (base, proj) = scratch_tree("api-key-flat");
        write_config(&proj, "api_key = \"flat-config-key\"\n");
        let run_config =
            resolve_run_config(config_inputs(&base, &proj, &[], None)).expect("resolve");
        assert_eq!(
            run_config
                .provider_config
                .provider(providers::OPENROUTER_ID)
                .and_then(|provider| provider.auth.api_key.as_deref()),
            Some("flat-config-key")
        );
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
    fn legacy_compaction_content_round_trips_summary_metadata() {
        let legacy = serialize_legacy_compaction_summary_content("summary text", "tail_42", true);
        let parsed: serde_json::Value = serde_json::from_str(&legacy).expect("legacy JSON");
        assert_eq!(parsed["type"], "compaction_summary");
        assert_eq!(parsed["summary"], "summary text");
        assert_eq!(parsed["tail_start_id"], "tail_42");
        assert_eq!(parsed["continued_request"], true);
        assert_eq!(
            legacy_compaction_summary_from_content(&legacy).as_deref(),
            Some("summary text")
        );
    }

    #[test]
    fn load_history_uses_summary_from_legacy_compaction_content_json() {
        let db = Database::open_in_memory().expect("memory db");
        let session = db.sessions().create(None).expect("create session");
        let legacy =
            serialize_legacy_compaction_summary_content("compacted summary", "tail_7", true);
        db.messages()
            .create(&session.id, "system", &legacy)
            .expect("persist legacy content message");

        let history = load_history(&db, &session.id).expect("history loads");
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].role, llm::Role::System);
        assert_eq!(history[0].content, "compacted summary");
    }

    #[test]
    fn load_history_preserves_json_shaped_user_and_assistant_content() {
        let db = Database::open_in_memory().expect("memory db");
        let session = db.sessions().create(None).expect("create session");
        let literal_json =
            serialize_legacy_compaction_summary_content("keep me verbatim", "tail_user", false);
        db.messages()
            .create(&session.id, "user", &literal_json)
            .expect("persist user message");
        db.messages()
            .create(&session.id, "assistant", &literal_json)
            .expect("persist assistant message");

        let history = load_history(&db, &session.id).expect("history loads");
        assert_eq!(history.len(), 2);
        assert_eq!(history[0].role, llm::Role::User);
        assert_eq!(history[0].content, literal_json.as_str());
        assert_eq!(history[1].role, llm::Role::Assistant);
        assert_eq!(history[1].content, literal_json.as_str());
    }

    #[test]
    fn load_history_does_not_decode_system_json_without_known_compaction_markers() {
        let db = Database::open_in_memory().expect("memory db");
        let session = db.sessions().create(None).expect("create session");
        let non_legacy = serde_json::json!({
            "type": "compaction_summary",
            "summary": "literal system payload"
        })
        .to_string();
        db.messages()
            .create(&session.id, "system", &non_legacy)
            .expect("persist system message");

        let history = load_history(&db, &session.id).expect("history loads");
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].role, llm::Role::System);
        assert_eq!(history[0].content, non_legacy);
    }

    #[test]
    fn load_history_prefers_compaction_summary_from_rich_compaction_parts() {
        let db = Database::open_in_memory().expect("memory db");
        let session = db.sessions().create(None).expect("create session");
        let part = MessagePart {
            id: String::new(),
            created_at: None,
            body: MessagePartBody::Compaction {
                summary: "summary from part".to_owned(),
                tail_start_id: "tail_9".to_owned(),
                continued_request: true,
            },
        };
        db.messages()
            .create_with_parts(&session.id, "system", "not legacy json", &[part])
            .expect("persist compaction part message");

        let history = load_history(&db, &session.id).expect("history loads");
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].role, llm::Role::System);
        assert_eq!(history[0].content, "summary from part");
    }

    #[test]
    fn memory_provenance_messages_are_excluded_from_model_history() {
        let db = Database::open_in_memory().expect("memory db");
        let session = db.sessions().create(None).expect("create session");
        let sources = vec![memory::MemorySource {
            path: std::path::PathBuf::from("/tmp/AGENTS.md"),
            kind: "AGENTS.md",
            content: "ignored".to_owned(),
        }];
        persist_memory_provenance(&db, &session.id, &sources);
        db.messages()
            .create(&session.id, "user", "hello")
            .expect("persist user message");

        let history = load_history(&db, &session.id).expect("history loads");
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].role, llm::Role::User);
        assert_eq!(history[0].content, "hello");
    }

    #[test]
    fn persist_memory_provenance_stores_source_metadata() {
        let db = Database::open_in_memory().expect("memory db");
        let session = db.sessions().create(None).expect("create session");
        let sources = vec![
            memory::MemorySource {
                path: std::path::PathBuf::from("/tmp/CLAUDE.md"),
                kind: "CLAUDE.md",
                content: "x".to_owned(),
            },
            memory::MemorySource {
                path: std::path::PathBuf::from("/tmp/AGENTS.md"),
                kind: "AGENTS.md",
                content: "y".to_owned(),
            },
        ];

        persist_memory_provenance(&db, &session.id, &sources);

        let messages = db
            .messages()
            .list_for_session(&session.id)
            .expect("list session messages");
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].role, "system");
        assert!(memory_provenance_from_content(&messages[0].content));
        let payload: serde_json::Value =
            serde_json::from_str(&messages[0].content).expect("parse provenance json");
        assert_eq!(payload["type"], "memory_provenance");
        let listed = payload["sources"].as_array().expect("sources array");
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0]["kind"], "CLAUDE.md");
        assert_eq!(listed[1]["kind"], "AGENTS.md");
    }

    #[test]
    fn persist_compactions_stores_legacy_content_json_and_compaction_parts() {
        let db = Database::open_in_memory().expect("memory db");
        let session = db.sessions().create(None).expect("create session");
        let outcome = RunOutcome {
            message: llm::Message::assistant("ignored"),
            usage: None,
            finish_reason: llm::FinishReason::Stop,
            steps: 1,
            compactions: vec![engine::CompactionRecord {
                summary: "durable summary".to_owned(),
                tail_start_id: "tail_11".to_owned(),
                continued_request: true,
                model: None,
            }],
            error: None,
        };

        persist_compactions(&db, &session.id, &outcome);

        let messages = db
            .messages()
            .list_for_session(&session.id)
            .expect("list session messages");
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].role, "system");
        let content: serde_json::Value =
            serde_json::from_str(&messages[0].content).expect("legacy content JSON");
        assert_eq!(content["type"], "compaction_summary");
        assert_eq!(content["summary"], "durable summary");
        assert_eq!(content["tail_start_id"], "tail_11");
        assert_eq!(content["continued_request"], true);
        assert_eq!(messages[0].parts.len(), 1);
        assert!(matches!(
            &messages[0].parts[0].body,
            MessagePartBody::Compaction {
                summary,
                tail_start_id,
                continued_request
            } if summary == "durable summary"
                && tail_start_id == "tail_11"
                && *continued_request
        ));
    }

    #[test]
    fn snapshot_part_summarizes_created_updated_and_deleted_paths_without_blob_oids() {
        let turn = checkpoints::RecordedTurn {
            label: "ses_snapshot".to_owned(),
            files: vec![
                checkpoints::TurnFileRecord {
                    rel_path: "new.txt".to_owned(),
                    pre: None,
                    post: Some(checkpoints::FileImage {
                        oid: "oid_created".to_owned(),
                        executable: false,
                    }),
                },
                checkpoints::TurnFileRecord {
                    rel_path: "edited.txt".to_owned(),
                    pre: Some(checkpoints::FileImage {
                        oid: "oid_before".to_owned(),
                        executable: false,
                    }),
                    post: Some(checkpoints::FileImage {
                        oid: "oid_after".to_owned(),
                        executable: false,
                    }),
                },
                checkpoints::TurnFileRecord {
                    rel_path: "removed.txt".to_owned(),
                    pre: Some(checkpoints::FileImage {
                        oid: "oid_deleted".to_owned(),
                        executable: false,
                    }),
                    post: None,
                },
            ],
        };

        let part = snapshot_part_from_turn("msg_turn_1", &turn).expect("snapshot part");
        assert_eq!(part.id, "part_msg_turn_1_snapshot_1");
        assert!(matches!(
            &part.body,
            MessagePartBody::Snapshot {
                label,
                created,
                updated,
                deleted
            } if label == "ses_snapshot"
                && created == &vec!["new.txt".to_owned()]
                && updated == &vec!["edited.txt".to_owned()]
                && deleted == &vec!["removed.txt".to_owned()]
        ));

        let json = serde_json::to_string(&part).expect("serialize snapshot part");
        assert!(
            !json.contains("oid_before")
                && !json.contains("oid_after")
                && !json.contains("oid_created")
                && !json.contains("oid_deleted"),
            "snapshot part must not expose git blob ids: {json}"
        );
    }

    #[test]
    fn snapshot_part_is_omitted_when_no_file_changes_were_recorded() {
        let turn = checkpoints::RecordedTurn {
            label: "ses_empty".to_owned(),
            files: Vec::new(),
        };
        assert!(snapshot_part_from_turn("msg_turn_empty", &turn).is_none());
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

    #[test]
    fn structured_schema_requires_json_output_format() {
        let err = parse_structured_schema(Some("{\"type\":\"object\"}"), OutputFormat::Text)
            .expect_err("non-json format must reject schema");
        assert!(err.contains("--format json"), "got: {err}");
        let parsed = parse_structured_schema(
            Some("{\"type\":\"object\",\"required\":[\"ok\"]}"),
            OutputFormat::Json,
        )
        .expect("schema parses")
        .expect("schema is present");
        assert_eq!(parsed["type"], "object");
    }

    #[test]
    fn structured_output_validation_is_typed_and_enforced() {
        let schema = serde_json::json!({
            "type": "object",
            "required": ["ok"],
            "properties": { "ok": { "type": "boolean" } },
            "additionalProperties": false
        });
        let parsed = validate_structured_output(r#"{"ok":true}"#, Some(&schema))
            .expect("valid output")
            .expect("parsed");
        assert_eq!(parsed["ok"], true);

        let err = validate_structured_output("not-json", Some(&schema))
            .expect_err("invalid JSON must fail with schema diagnostic");
        assert_eq!(err.kind(), "schema-validation");

        let err = validate_structured_output(r#"{"ok":"yes"}"#, Some(&schema))
            .expect_err("type mismatch must fail");
        assert_eq!(err.kind(), "schema-validation");
    }

    /// A completed-turn outcome with assistant text and integer usage, mirroring a real turn.
    fn sample_outcome() -> RunOutcome {
        RunOutcome {
            message: llm::Message::assistant("Hello! How can I help you today?"),
            usage: Some(llm::Usage {
                input_tokens: 9,
                output_tokens: 8,
                total_tokens: 17,
                cache_creation_input_tokens: None,
                cache_read_input_tokens: None,
                reasoning_output_tokens: None,
                cost: Some(0.0000056),
            }),
            finish_reason: llm::FinishReason::Stop,
            steps: 1,
            compactions: Vec::new(),
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
            None,
            "ses_x",
            &[],
            &[],
            &sample_outcome(),
            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            None,
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
            None,
            "ses_abc123",
            &[],
            &[],
            &sample_outcome(),
            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            None,
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
            None,
            "ses_cumulative",
            &[],
            &[],
            &sample_outcome(),
            Some(session_usage),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            None,
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
        let rendered = render_json_document(
            "m",
            "general",
            None,
            "ses_y",
            &[],
            &[],
            &outcome,
            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            None,
        )
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
        let line = render_session_line("plan", None, "ses_seed42", &tools, &[]).expect("render");
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
        let defs = vec![ToolDef {
            name: "task".to_owned(),
            description: "Delegate".to_owned(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "agent": { "enum": ["explore"] }
                }
            }),
        }];
        let rendered = render_json_document(
            "openai/gpt-4o-mini",
            "build",
            None,
            "ses_gpt",
            &gpt_tools,
            &defs,
            &sample_outcome(),
            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            None,
        )
        .expect("render");
        let value: serde_json::Value = serde_json::from_str(&rendered).expect("json");
        let tools = value["tools"].as_array().expect("tools is an array");
        let names: Vec<&str> = tools.iter().filter_map(serde_json::Value::as_str).collect();
        assert!(names.contains(&"apply_patch"), "got: {names:?}");
        assert!(!names.contains(&"edit"), "got: {names:?}");
        assert!(!names.contains(&"write"), "got: {names:?}");
        assert_eq!(value["tool_definitions"][0]["name"], "task");
        assert_eq!(
            value["tool_definitions"][0]["parameters"]["properties"]["agent"]["enum"],
            serde_json::json!(["explore"])
        );
    }

    #[test]
    fn json_document_carries_rich_message_parts_when_present() {
        let parts = vec![
            MessagePart {
                id: "msg_demo_p0001".to_owned(),
                created_at: Some("2026-01-01 00:00:00".to_owned()),
                body: MessagePartBody::Agent {
                    name: "build".to_owned(),
                    mode: None,
                    model: Some("google/gemini-2.5-flash".to_owned()),
                },
            },
            MessagePart::text("answer body"),
        ];
        let rendered = render_json_document(
            "google/gemini-2.5-flash",
            "build",
            None,
            "ses_parts",
            &[],
            &[],
            &sample_outcome(),
            None,
            Vec::new(),
            Vec::new(),
            parts,
            None,
        )
        .expect("render");
        let value: serde_json::Value = serde_json::from_str(&rendered).expect("json");
        assert_eq!(value["parts"][0]["type"], "agent");
        assert_eq!(value["parts"][0]["id"], "msg_demo_p0001");
        assert_eq!(value["parts"][1]["type"], "text");
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
                cache_creation_input_tokens: None,
                cache_read_input_tokens: None,
                reasoning_output_tokens: None,
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

    /// Remove every SGR escape sequence so a highlighted string can be compared to its plain form.
    fn strip_ansi(s: &str) -> String {
        let mut out = String::with_capacity(s.len());
        let mut chars = s.chars();
        while let Some(c) = chars.next() {
            if c == '\x1b' {
                for e in chars.by_ref() {
                    if e == 'm' {
                        break;
                    }
                }
            } else {
                out.push(c);
            }
        }
        out
    }

    // The transformation `emit_json` applies on the human/TTY branch: pretty-print + highlight the
    // serialized document. This mirrors the exact `emit_json` write logic without a live turn.
    fn json_tty_render(styler: style::Styler, line: &str) -> String {
        if styler.is_enabled() {
            format!(
                "{}\n",
                styler.highlight_json_pretty(line.trim_end_matches('\n'))
            )
        } else {
            line.to_owned()
        }
    }

    // VAL-CLI-007: off a TTY the `--format json` bytes are byte-identical to the serialized
    // document (no pretty-print, no ANSI) — the styler is a pure passthrough.
    #[test]
    fn json_piped_bytes_are_byte_identical_to_the_serialized_document() {
        let line = render_json_document(
            "google/gemini-2.5-flash",
            "build",
            None,
            "ses_pipe",
            &["read".to_owned(), "edit".to_owned()],
            &[],
            &sample_outcome(),
            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            None,
        )
        .expect("render");
        let piped = json_tty_render(style::Styler::new(false), &line);
        assert_eq!(
            piped, line,
            "piped json is the untouched serialized document"
        );
        assert!(!piped.contains('\x1b'), "piped json carries no ANSI byte");
        assert_eq!(piped.matches('\n').count(), 1, "piped json is one line");
    }

    // VAL-CLI-004/006: on a TTY the `--format json` text is multi-line, 2-space-indented, and
    // syntax-highlighted, yet stripped of ANSI it reparses to the SAME JSON value as the piped
    // document (only insignificant whitespace was added; keys/order preserved).
    #[test]
    fn json_tty_is_pretty_highlighted_and_reparses_to_the_same_value() {
        let line = render_json_document(
            "google/gemini-2.5-flash",
            "build",
            None,
            "ses_tty",
            &["read".to_owned(), "edit".to_owned()],
            &[],
            &sample_outcome(),
            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            None,
        )
        .expect("render");
        let tty = json_tty_render(style::Styler::new(true), &line);
        assert!(tty.contains('\x1b'), "tty json is syntax-highlighted");
        assert!(
            tty.matches('\n').count() > 1,
            "tty json is pretty-printed across multiple lines"
        );
        let plain = strip_ansi(&tty);
        let a: serde_json::Value = serde_json::from_str(&line).expect("piped json");
        let b: serde_json::Value = serde_json::from_str(&plain).expect("tty json reparses");
        assert_eq!(a, b, "pretty TTY reflow preserves the exact JSON value");
        // VAL-CLI-005: the finish_reason "stop" value is status-green on the TTY.
        let style::Rgb(r, g, b) = style::OK_GREEN;
        let green = format!("\x1b[38;2;{r};{g};{b}m");
        assert!(
            tty.contains(&format!("{green}\"stop\"\x1b[0m")),
            "the stop finish_reason renders green"
        );
    }

    // VAL-CLI-011: off a TTY each `--jsonl` line is byte-identical (no ANSI) and no footer is
    // written; on a TTY the same lines are highlighted and a `stream complete · N events` footer
    // follows. Mirrors the `write_jsonl_line` + footer logic without a live turn.
    #[test]
    fn jsonl_piped_is_raw_bytes_no_footer_tty_adds_footer() {
        let events = [
            render_session_line("build", None, "ses_j", &["edit".to_owned()], &[])
                .expect("session"),
            render_event_line(&RunnerEvent::TextDelta {
                text: "hi".to_owned(),
            })
            .expect("delta"),
            render_event_line(&RunnerEvent::Done {
                reason: llm::FinishReason::Stop,
                steps: 1,
            })
            .expect("done"),
        ];

        // Piped: passthrough, no footer.
        let disabled = style::Styler::new(false);
        let mut piped = Vec::new();
        let mut count = 0usize;
        for line in &events {
            write_jsonl_line(&mut piped, disabled, line, &mut count);
        }
        let piped_text = String::from_utf8(piped).expect("utf8");
        let expected: String = events.iter().map(|l| format!("{l}\n")).collect();
        assert_eq!(
            piped_text, expected,
            "piped jsonl is the raw serialized lines"
        );
        assert!(!piped_text.contains('\x1b'), "piped jsonl carries no ANSI");
        assert!(
            !piped_text.contains("stream complete"),
            "no TTY footer when piped"
        );
        assert_eq!(count, 3, "event counter tracks each written line");

        // TTY: highlighted lines + footer with the real event count.
        let enabled = style::Styler::new(true);
        let mut tty = Vec::new();
        let mut tty_count = 0usize;
        for line in &events {
            write_jsonl_line(&mut tty, enabled, line, &mut tty_count);
        }
        let footer = format!("stream complete {} {tty_count} events", style::SEPARATOR);
        tty.extend_from_slice(enabled.paint(&footer, style::FAINT).as_bytes());
        tty.push(b'\n');
        let tty_text = String::from_utf8(tty).expect("utf8");
        assert!(tty_text.contains('\x1b'), "tty jsonl lines are highlighted");
        assert!(
            tty_text.contains("stream complete · 3 events"),
            "footer names the real event count, got: {tty_text:?}"
        );
        // Each highlighted line, stripped of ANSI, equals its raw serialized line (one per line).
        for line in &events {
            assert!(
                strip_ansi(&tty_text).contains(line),
                "raw event line survives highlighting intact"
            );
        }
    }
}
