//! The durable subagent child turn: the real work behind the `task` tool.
//!
//! The `tools` crate owns the model-facing `task` tool, while this runner owns the real durable
//! child turn: session creation, prompt/persona assembly, permission narrowing, child tool
//! materializing, checkpoint/formatter/LSP wiring, and exact-PID cleanup of any processes the child
//! tools spawn.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use db::{Database, Session};
use engine::{
    CompactionPolicy, EnvironmentBlock, HistoryMessage, RunOutcome, RunnerEvent, SessionRunner,
    SystemContext, Turn, project_history,
};
use events::EventBus;
use futures_util::future::BoxFuture;
use permissions::{
    Action, Decision, DecisionOverride, PermissionEngine, PermissionRequest, RuleError, Ruleset,
    load_agent_rulesets,
};
use settings::ConfigInputs;
use tools::{TaskRequest, TaskRunner, ToolOutput};

use crate::bridges::{
    self, ChildDispatcher, DbTodoStore, LspHooks, PostMutationHooks, RecorderBridge,
};
use crate::custom_agents::ActiveAgent;
use crate::provider::{AuditedProvider, ChildProviderWrapper, ProviderAuditConfig};

/// Runtime configuration captured from the parent turn and reused by child tasks.
#[derive(Clone)]
pub struct CliTaskRunnerConfig {
    pub provider: Arc<dyn llm::Provider>,
    pub provider_audit: Option<ProviderAuditConfig>,
    /// Surface-supplied provider wrapper (e.g. plugin request-shaping). `None` on surfaces without
    /// plugin request middleware (the TUI), so the child provider is used as resolved.
    pub plugin_provider: Option<Arc<dyn ChildProviderWrapper>>,
    pub audit: Arc<audit::AuditSink>,
    pub db_path: PathBuf,
    pub cwd: PathBuf,
    pub parent_session_id: String,
    pub parent_cap: Option<Ruleset>,
    /// Parent-surface dynamic permission hook. A child consults only its denied decisions, so a
    /// plugin denial that applies to the parent remains a cap without permitting it to weaken the
    /// child's own persona or ruleset restrictions.
    pub parent_permission_override: Option<Arc<dyn DecisionOverride>>,
    pub parent_model_id: String,
    pub formatter_config: format::FormatterConfig,
    pub lsp_config: lsp::LspConfigSet,
    pub config_inputs: ConfigInputs,
    pub project_memory: Option<String>,
    pub bus: Arc<EventBus>,
}

pub struct CliTaskRunner {
    pub(crate) config: CliTaskRunnerConfig,
    pub(crate) agents: BTreeMap<String, ActiveAgent>,
    pub(crate) background_tasks: Arc<Mutex<BTreeMap<String, BackgroundTaskEntry>>>,
}

pub(crate) struct BackgroundTaskEntry {
    agent: String,
    description: Option<String>,
    command: Option<String>,
    state: BackgroundTaskState,
}

enum BackgroundTaskState {
    Running(Option<tokio::task::JoinHandle<ToolOutput>>),
    Finished(ToolOutput),
}

impl TaskRunner for CliTaskRunner {
    fn run_task<'a>(&'a self, request: TaskRequest) -> BoxFuture<'a, ToolOutput> {
        Box::pin(async move { self.run(request).await })
    }
}

impl CliTaskRunner {
    async fn run(&self, request: TaskRequest) -> ToolOutput {
        if request.background {
            return self.run_background(request).await;
        }

        if let Some(task_id) = request.task_id.clone() {
            if request.prompt.is_none() {
                return self.rejoin_task(&task_id).await;
            }
        }

        self.run_foreground(request).await
    }

    /// Drive every still-running background subagent child turn to completion before the parent
    /// process exits.
    ///
    /// A background `task` is a `tokio::spawn`ed child turn on the parent's current-thread runtime:
    /// once the parent turn's `block_on` returns, that child is no longer polled and would be
    /// aborted when the runtime is dropped, losing its persisted assistant answer. Awaiting the
    /// outstanding join handles here (analogous to reaping background `bash` jobs at shutdown)
    /// guarantees the child transcript is durably written, so the task remains rejoinable by id and
    /// visible in subagent surfaces after a restart, with no orphaned work left behind. The child
    /// turn is itself bounded (MAX_STEPS + transport idle timeouts), so this drain terminates.
    pub async fn drain_background(&self) {
        let handles: Vec<(String, tokio::task::JoinHandle<ToolOutput>)> = {
            let mut map = self.background_tasks.lock().expect("background lock");
            map.iter_mut()
                .filter_map(|(id, entry)| match &mut entry.state {
                    BackgroundTaskState::Running(handle) => {
                        handle.take().map(|handle| (id.clone(), handle))
                    }
                    BackgroundTaskState::Finished(_) => None,
                })
                .collect()
        };

        for (task_id, handle) in handles {
            let output = match handle.await {
                Ok(output) => output,
                Err(err) => ToolOutput::error(format!(
                    "task: background subagent task {task_id} failed to join: {err}"
                )),
            };
            if let Some(entry) = self
                .background_tasks
                .lock()
                .expect("background lock")
                .get_mut(&task_id)
            {
                entry.state = BackgroundTaskState::Finished(output);
            }
        }
    }

    async fn run_background(&self, request: TaskRequest) -> ToolOutput {
        let Some(agent_name) = request.agent.clone() else {
            return ToolOutput::error(
                "task: the 'subagent_type' argument is required when background=true",
            );
        };
        let Some(agent) = self.agents.get(&agent_name) else {
            return ToolOutput::error(format!("task: subagent `{agent_name}` is not available"));
        };
        let agent_name = agent.name().to_owned();
        let Some(_prompt) = request.prompt.clone() else {
            return ToolOutput::error(
                "task: the 'prompt' argument is required when background=true",
            );
        };

        let db = match Database::open(&self.config.db_path) {
            Ok(db) => db,
            Err(err) => return ToolOutput::error(format!("task: failed to open database: {err}")),
        };
        let (child_session, _resumed) = match self.resolve_child_session(&db, &request, &agent_name)
        {
            Ok(value) => value,
            Err(err) => return err,
        };
        let task_id = child_session.id.clone();

        if let Some(result) = self.poll_background_task(&task_id).await {
            return result;
        }

        let mut child_request = request.clone();
        child_request.task_id = Some(task_id.clone());
        child_request.background = false;

        let config = self.config.clone();
        let agents = self.agents.clone();
        let background_tasks = Arc::clone(&self.background_tasks);
        let handle = tokio::spawn(async move {
            let runner = CliTaskRunner {
                config,
                agents,
                background_tasks,
            };
            runner.run_foreground(child_request).await
        });

        self.background_tasks
            .lock()
            .expect("background lock")
            .insert(
                task_id.clone(),
                BackgroundTaskEntry {
                    agent: agent_name.clone(),
                    description: request.description.clone(),
                    command: request.command.clone(),
                    state: BackgroundTaskState::Running(Some(handle)),
                },
            );

        ToolOutput::text(render_task_running_output(
            &task_id,
            &agent_name,
            request.description.as_deref(),
            request.command.as_deref(),
        ))
    }

    async fn rejoin_task(&self, task_id: &str) -> ToolOutput {
        if let Some(result) = self.poll_background_task(task_id).await {
            return result;
        }

        let db = match Database::open(&self.config.db_path) {
            Ok(db) => db,
            Err(err) => return ToolOutput::error(format!("task: failed to open database: {err}")),
        };
        let session = match db.sessions().require(task_id) {
            Ok(session) => session,
            Err(err) => return ToolOutput::error(format!("task: {err}")),
        };
        let agent = match self.require_owned_subagent_session(&session, None) {
            Ok(agent) => agent,
            Err(err) => return err,
        };
        let messages = match db.messages().list_for_session(task_id) {
            Ok(messages) => messages,
            Err(err) => return ToolOutput::error(format!("task: {err}")),
        };
        let Some(answer) = messages
            .iter()
            .rev()
            .find(|message| message.role == "assistant")
            .map(|message| message.content.clone())
        else {
            return ToolOutput::text(render_task_running_output(task_id, &agent, None, None));
        };
        ToolOutput::text(render_task_completed_output(RenderTaskCompletedOutput {
            task_id,
            agent: &agent,
            parent_session_id: &self.config.parent_session_id,
            tool_summary: "unknown",
            answer: &answer,
            tool_detail_section: "",
            resumed: false,
            description: None,
            command: None,
        }))
    }

    async fn poll_background_task(&self, task_id: &str) -> Option<ToolOutput> {
        let join_to_await = {
            let mut map = self.background_tasks.lock().expect("background lock");
            let entry = map.get_mut(task_id)?;
            match &mut entry.state {
                BackgroundTaskState::Finished(output) => {
                    return Some(output.clone());
                }
                BackgroundTaskState::Running(handle) => {
                    let Some(current) = handle.as_ref() else {
                        return Some(ToolOutput::error(format!(
                            "task: background task {task_id} is in an invalid state"
                        )));
                    };
                    if current.is_finished() {
                        handle.take()
                    } else {
                        return Some(ToolOutput::text(render_task_running_output(
                            task_id,
                            &entry.agent,
                            entry.description.as_deref(),
                            entry.command.as_deref(),
                        )));
                    }
                }
            }
        };

        let handle = join_to_await.expect("finished handle present");
        let output = match handle.await {
            Ok(output) => output,
            Err(err) => ToolOutput::error(format!(
                "task: background subagent task {task_id} failed to join: {err}"
            )),
        };
        let mut map = self.background_tasks.lock().expect("background lock");
        if let Some(entry) = map.get_mut(task_id) {
            entry.state = BackgroundTaskState::Finished(output.clone());
        } else {
            map.insert(
                task_id.to_owned(),
                BackgroundTaskEntry {
                    agent: "unknown".to_owned(),
                    description: None,
                    command: None,
                    state: BackgroundTaskState::Finished(output.clone()),
                },
            );
        }
        Some(output)
    }

    async fn run_foreground(&self, request: TaskRequest) -> ToolOutput {
        let Some(agent_name) = request.agent.clone() else {
            return ToolOutput::error(
                "task: the 'subagent_type' argument is required when 'prompt' is provided",
            );
        };
        let Some(agent) = self.agents.get(&agent_name).cloned() else {
            return ToolOutput::error(format!("task: subagent `{agent_name}` is not available"));
        };
        let agent_name = agent.name().to_owned();
        let Some(prompt) = request.prompt.clone() else {
            return ToolOutput::error("task: the 'prompt' argument is required");
        };

        let db = match Database::open(&self.config.db_path) {
            Ok(db) => db,
            Err(err) => return ToolOutput::error(format!("task: failed to open database: {err}")),
        };
        let (child_session, resumed) = match self.resolve_child_session(&db, &request, &agent_name)
        {
            Ok(value) => value,
            Err(err) => return err,
        };

        let child_audit = Arc::new(self.config.audit.child_session(child_session.id.clone()));
        let provider = self.provider_for_child(child_audit.clone());

        if let Err(err) = db.messages().create(&child_session.id, "user", &prompt) {
            return ToolOutput::error(format!(
                "task: failed to persist child prompt in {}: {err}",
                child_session.id
            ));
        }

        let model_id = agent
            .model()
            .map(str::to_owned)
            .unwrap_or_else(|| self.config.parent_model_id.clone());
        if let Err(message) = providers::validate_model_id(&model_id) {
            return ToolOutput::error(format!("task: {message}"));
        }
        let resolved_model = providers::resolve_model(Some(&model_id));
        let mut turn = Turn::new(
            &model_id,
            render_system_context(&model_id, &agent, self.config.project_memory.clone()),
            prompt,
        )
        .with_history(load_history(&db, &child_session.id).unwrap_or_default())
        .with_compaction(compaction_policy(&resolved_model));
        if let Some(temperature) = agent.temperature() {
            turn = turn.with_temperature(temperature);
        }

        let todo_store: Arc<dyn tools::TodoStore> =
            Arc::new(DbTodoStore::new(self.config.db_path.clone()));
        let mut dispatcher = ChildDispatcher::for_model(
            self.config.cwd.clone(),
            &model_id,
            child_session.id.clone(),
            todo_store,
        )
        .with_audit(child_audit);

        let child_permissions = match self.child_permissions(&agent) {
            Ok(engine) => engine,
            Err(err) => return ToolOutput::error(format!("task: {err}")),
        };
        if let Some(engine) = child_permissions {
            dispatcher = dispatcher.with_permissions(engine);
        }

        let checkpoint_recorder = match bridges::open_store(&self.config.cwd) {
            Ok(Some(store)) => Some(Arc::new(store.begin_turn(child_session.id.clone()))),
            Ok(None) => None,
            Err(err) => {
                eprintln!("cortex run: subagent checkpoint store unavailable: {err}");
                None
            }
        };
        if let Some(recorder) = &checkpoint_recorder {
            dispatcher =
                dispatcher.with_checkpoints(Arc::new(RecorderBridge::new(recorder.clone())));
        }

        let lsp_hooks = Arc::new(LspHooks::new(
            self.config.cwd.clone(),
            self.config.lsp_config.clone(),
        ));
        let mutation_hooks = Arc::new(PostMutationHooks::with_config(
            self.config.cwd.clone(),
            self.config.formatter_config.clone(),
            lsp_hooks.clone(),
        ));
        dispatcher = dispatcher
            .with_hooks(mutation_hooks)
            .with_lsp_service(lsp_hooks.clone());

        let mut tool_names = Vec::new();
        let mut tool_details = Vec::new();
        let outcome = SessionRunner::default()
            .run_with_tools(provider.as_ref(), turn, &dispatcher, |event| {
                if let RunnerEvent::ToolResult { name, parts, .. } = event {
                    let text = parts
                        .iter()
                        .filter_map(protocol::ToolResultPart::as_text)
                        .collect::<Vec<_>>()
                        .join("\n");
                    if !text.trim().is_empty() {
                        tool_details.push(format!(
                            "{}: {}",
                            name,
                            truncate_tool_detail(&text, 800)
                        ));
                    }
                    tool_names.push(name);
                }
            })
            .await;

        dispatcher.background_jobs().shutdown().await;
        lsp_hooks.shutdown();
        if let Some(recorder) = &checkpoint_recorder {
            let _ = bridges::finalize_turn(recorder);
        }

        persist_usage(&db, &child_session.id, &outcome);
        if let Some(err) = &outcome.error {
            return ToolOutput::error(format!(
                "task_id: {}\nstate: error\nsubagent_type: {}\nSubagent `{}` failed in session `{}`: {}",
                child_session.id, agent_name, agent_name, child_session.id, err
            ));
        }
        persist_assistant(&db, &child_session.id, &outcome);

        let tool_summary = if tool_names.is_empty() {
            "no tools".to_owned()
        } else {
            tool_names.join(", ")
        };
        let tool_detail_section = if tool_details.is_empty() {
            String::new()
        } else {
            format!("\n\nChild tool results:\n{}", tool_details.join("\n---\n"))
        };
        ToolOutput::text(render_task_completed_output(RenderTaskCompletedOutput {
            task_id: &child_session.id,
            agent: &agent_name,
            parent_session_id: &self.config.parent_session_id,
            tool_summary: &tool_summary,
            answer: outcome.answer(),
            tool_detail_section: &tool_detail_section,
            resumed,
            description: request.description.as_deref(),
            command: request.command.as_deref(),
        }))
    }

    fn resolve_child_session(
        &self,
        db: &Database,
        request: &TaskRequest,
        agent_name: &str,
    ) -> Result<(Session, bool), ToolOutput> {
        if let Some(task_id) = request.task_id.as_deref() {
            let session = db
                .sessions()
                .require(task_id)
                .map_err(|err| ToolOutput::error(format!("task: {err}")))?;
            self.require_owned_subagent_session(&session, Some(agent_name))?;
            return Ok((session, true));
        }
        let title = request
            .description
            .as_deref()
            .map(str::trim)
            .filter(|description| !description.is_empty());
        db.sessions()
            .create_subagent_child(&self.config.parent_session_id, agent_name, title)
            .map(|session| (session, false))
            .map_err(|err| {
                ToolOutput::error(format!("task: failed to create child session: {err}"))
            })
    }

    /// Confirm that a `task_id` belongs to this parent and retains the agent persona it was
    /// created under. A task id is durable but is never a cross-session capability token.
    fn require_owned_subagent_session(
        &self,
        session: &Session,
        expected_agent: Option<&str>,
    ) -> Result<String, ToolOutput> {
        if session.parent_id.as_deref() != Some(self.config.parent_session_id.as_str()) {
            return Err(ToolOutput::error(format!(
                "task: session `{}` does not belong to parent session `{}`",
                session.id, self.config.parent_session_id
            )));
        }
        let Some(agent) = session
            .subagent_type
            .as_deref()
            .filter(|agent| !agent.is_empty())
            .map(ToOwned::to_owned)
        else {
            return Err(ToolOutput::error(format!(
                "task: session `{}` is not a valid subagent task",
                session.id
            )));
        };
        if !self.agents.contains_key(&agent) {
            return Err(ToolOutput::error(format!(
                "task: subagent `{agent}` is not available in this parent turn"
            )));
        }
        if let Some(expected_agent) = expected_agent
            && agent != expected_agent
        {
            return Err(ToolOutput::error(format!(
                "task: session `{}` runs as subagent `{agent}`, not `{expected_agent}`",
                session.id
            )));
        }
        Ok(agent)
    }

    fn child_permissions(
        &self,
        agent: &ActiveAgent,
    ) -> Result<Option<Arc<PermissionEngine>>, RuleError> {
        let builtin = bridges::builtin_agent_permission_rules(agent.base_agent());
        let mut hard_deny_rules = builtin.restriction_rules.clone();
        let config_ruleset =
            load_agent_rulesets(&self.config.config_inputs)?.for_agent(agent.name());
        let mut ruleset = Ruleset::new(builtin.base_rules);
        for rule in config_ruleset.rules() {
            ruleset.push(rule.clone());
        }
        for rule in agent.permissions() {
            ruleset.push(rule.clone());
        }
        for rule in builtin.restriction_rules {
            ruleset.push(rule);
        }
        let trust_level = policy::resolve_trust(&self.config.config_inputs)?;
        for rule in policy::trust_ruleset(trust_level).rules() {
            if rule.action == Action::Deny {
                hard_deny_rules.push(rule.clone());
            } else {
                ruleset.push(rule.clone());
            }
        }

        let has_parent_cap = self.config.parent_cap.is_some();
        if ruleset.is_empty() && hard_deny_rules.is_empty() && !has_parent_cap {
            return Ok(None);
        }
        let mut engine = PermissionEngine::new(ruleset)
            .with_hard_denies(Ruleset::new(hard_deny_rules))
            .with_bus(self.config.bus.clone());
        if let Some(parent_cap) = &self.config.parent_cap {
            engine = engine.with_parent_cap(parent_cap.clone());
        }
        if let (Some(parent_cap), Some(parent_override)) = (
            &self.config.parent_cap,
            &self.config.parent_permission_override,
        ) {
            engine = engine.with_override(Arc::new(ParentDenyOverride {
                parent_cap: parent_cap.clone(),
                parent_override: parent_override.clone(),
            }));
        }
        Ok(Some(Arc::new(engine)))
    }

    fn provider_for_child(&self, child_audit: Arc<audit::AuditSink>) -> Arc<dyn llm::Provider> {
        let base = match &self.config.provider_audit {
            Some(config) => Arc::new(AuditedProvider::new(
                self.config.provider.clone(),
                child_audit.clone(),
                config.provider.clone(),
                config.endpoint.clone(),
            )) as Arc<dyn llm::Provider>,
            None => self.config.provider.clone(),
        };
        match &self.config.plugin_provider {
            Some(wrapper) => wrapper.wrap(base, &child_audit),
            None => base,
        }
    }
}

/// Applies only dynamic parent denials to a child. Dynamic approvals must not weaken the child's
/// persona, hard-deny, or configured rules, while a denial still closes the parent-to-child
/// permission gap.
struct ParentDenyOverride {
    parent_cap: Ruleset,
    parent_override: Arc<dyn DecisionOverride>,
}

impl DecisionOverride for ParentDenyOverride {
    fn override_decision(
        &self,
        request: &PermissionRequest,
        _proposed: Action,
    ) -> Option<Decision> {
        let parent_action = self.parent_cap.classify(&request.tool, &request.subject);
        matches!(
            self.parent_override
                .override_decision(request, parent_action),
            Some(Decision::Deny)
        )
        .then_some(Decision::Deny)
    }
}

/// The compaction policy for a child turn, derived from the resolved model's context window and
/// per-turn output limit. The effective window can be overridden via `CORTEX_CONTEXT_LIMIT_TOKENS`
/// (an operational tuning/testing knob); an unknown model falls back to the engine's default
/// window. Kept identical to the headless run's parent-turn policy so subagent budgets match.
#[must_use]
fn compaction_policy(model: &providers::ResolvedModel) -> CompactionPolicy {
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

fn context_limit_override() -> Option<usize> {
    parse_context_limit(std::env::var("CORTEX_CONTEXT_LIMIT_TOKENS").ok().as_deref())
}

fn parse_context_limit(raw: Option<&str>) -> Option<usize> {
    let parsed: usize = raw?.trim().parse().ok()?;
    (parsed >= 1).then_some(parsed)
}

fn render_task_running_output(
    task_id: &str,
    agent: &str,
    description: Option<&str>,
    command: Option<&str>,
) -> String {
    let mut lines = vec![
        format!("task_id: {task_id}"),
        "state: running".to_owned(),
        format!("subagent_type: {agent}"),
    ];
    if let Some(description) = description.filter(|value| !value.trim().is_empty()) {
        lines.push(format!("description: {}", description.trim()));
    }
    if let Some(command) = command.filter(|value| !value.trim().is_empty()) {
        lines.push(format!("command: {}", command.trim()));
    }
    lines.push("The subagent task is running in the background. Re-call the task tool with this task_id to rejoin.".to_owned());
    lines.join("\n")
}

struct RenderTaskCompletedOutput<'a> {
    task_id: &'a str,
    agent: &'a str,
    parent_session_id: &'a str,
    tool_summary: &'a str,
    answer: &'a str,
    tool_detail_section: &'a str,
    resumed: bool,
    description: Option<&'a str>,
    command: Option<&'a str>,
}

fn render_task_completed_output(input: RenderTaskCompletedOutput<'_>) -> String {
    let mut lines = vec![
        format!("task_id: {}", input.task_id),
        "state: completed".to_owned(),
        format!("resumed: {}", input.resumed),
        format!("subagent_type: {}", input.agent),
    ];
    if let Some(description) = input.description.filter(|value| !value.trim().is_empty()) {
        lines.push(format!("description: {}", description.trim()));
    }
    if let Some(command) = input.command.filter(|value| !value.trim().is_empty()) {
        lines.push(format!("command: {}", command.trim()));
    }
    lines.push(format!(
        "Subagent `{}` completed in session `{}` (parent `{}`; tools: {}).",
        input.agent, input.task_id, input.parent_session_id, input.tool_summary
    ));
    lines.push(String::new());
    lines.push(input.answer.to_owned());
    if !input.tool_detail_section.is_empty() {
        lines.push(input.tool_detail_section.to_owned());
    }
    lines.join("\n")
}

fn render_system_context(
    model_id: &str,
    agent: &ActiveAgent,
    project_memory: Option<String>,
) -> String {
    let mut system = SystemContext::for_model(model_id, EnvironmentBlock::from_process())
        .with_project_memory(project_memory)
        .with_agent(agent.base_agent())
        .render();
    if let Some(prompt) = agent.custom_prompt() {
        system.push_str("\n\nCustom subagent instructions:\n");
        system.push_str(prompt);
    }
    system
}

fn load_history(db: &Database, session_id: &str) -> Result<Vec<llm::Message>, db::DbError> {
    let history = db
        .messages()
        .list_for_session(session_id)?
        .into_iter()
        .map(|message| HistoryMessage::new(message.role, message.content))
        .collect::<Vec<_>>();
    Ok(project_history(&history))
}

fn persist_assistant(db: &Database, session_id: &str, outcome: &RunOutcome) {
    if let Err(err) = db
        .messages()
        .create(session_id, "assistant", outcome.answer())
    {
        eprintln!("cortex run: failed to persist the subagent assistant reply: {err}");
    }
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
        eprintln!("cortex run: failed to persist subagent usage: {err}");
    }
}

fn truncate_tool_detail(text: &str, max_chars: usize) -> String {
    let mut out = String::new();
    for ch in text.chars().take(max_chars) {
        out.push(ch);
    }
    if text.chars().count() > max_chars {
        out.push_str("\n[truncated child tool result]");
    }
    out
}
