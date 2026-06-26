//! Headless `task` subagent execution.
//!
//! The `tools` crate owns the model-facing `task` tool, while the CLI owns the real durable child
//! turn: session creation, prompt/persona assembly, permission narrowing, child tool materializing,
//! checkpoint/formatter/LSP wiring, and exact-PID cleanup of any processes the child tools spawn.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use db::{Database, Session};
use engine::{
    EnvironmentBlock, HistoryMessage, RunOutcome, RunnerEvent, SessionRunner, SystemContext, Turn,
    project_history,
};
use events::EventBus;
use futures_util::future::BoxFuture;
use permissions::{Action, PermissionEngine, RuleError, Ruleset, load_agent_rulesets};
use settings::ConfigInputs;
use tools::{TaskRequest, TaskRunner, TaskTarget, ToolOutput};

use crate::custom_agents::{self, ActiveAgent};
use crate::plugin_bridge::PluginState;
use crate::tooling::RegistryDispatcher;

/// The filtered subagent catalogue attached to one parent turn.
pub(crate) struct SubagentCatalog {
    /// Model-facing targets, already filtered by `task:<agent>` permission.
    pub targets: Vec<TaskTarget>,
    agents: BTreeMap<String, ActiveAgent>,
}

impl SubagentCatalog {
    /// Build the task runner for this catalogue.
    pub fn runner(&self, config: CliTaskRunnerConfig) -> Arc<dyn TaskRunner> {
        Arc::new(CliTaskRunner {
            config,
            agents: self.agents.clone(),
            background_tasks: Arc::new(Mutex::new(BTreeMap::new())),
        })
    }
}

/// Build the subagent catalogue for the current parent.
///
/// Built-in `build` is excluded because it is the primary/default agent. Custom agents are offered
/// only when their markdown mode is `subagent` or `all`. The final list is filtered through the
/// parent permission engine: with no engine every subagent is allowed; with an engine only
/// `task:<agent>` actions classified as `allow` are advertised.
#[must_use]
pub(crate) fn permitted_catalog(
    discovery: &custom_agents::Discovery,
    permission_engine: Option<&Arc<PermissionEngine>>,
) -> SubagentCatalog {
    let mut targets = Vec::new();
    let mut agents = BTreeMap::new();

    for agent in engine::Agent::SUBAGENTS {
        if task_allowed(permission_engine, agent.name()) {
            targets.push(TaskTarget::new(agent.name(), agent.description()));
            agents.insert(agent.name().to_owned(), ActiveAgent::Builtin(agent));
        }
    }

    for agent in discovery
        .agents
        .values()
        .filter(|agent| agent.availability.spawnable())
    {
        if task_allowed(permission_engine, &agent.name) {
            targets.push(TaskTarget::new(&agent.name, &agent.description));
            agents.insert(agent.name.clone(), ActiveAgent::Custom(agent.clone()));
        }
    }

    SubagentCatalog { targets, agents }
}

fn task_allowed(engine: Option<&Arc<PermissionEngine>>, agent_name: &str) -> bool {
    engine.is_none_or(|engine| engine.classify("task", agent_name) == Action::Allow)
}

/// Runtime configuration captured from the parent turn and reused by child tasks.
#[derive(Clone)]
pub(crate) struct CliTaskRunnerConfig {
    pub provider: Arc<dyn llm::Provider>,
    pub provider_audit: Option<ProviderAuditConfig>,
    pub plugin_state: PluginState,
    pub audit: Arc<audit::AuditSink>,
    pub db_path: PathBuf,
    pub cwd: PathBuf,
    pub parent_session_id: String,
    pub parent_cap: Option<Ruleset>,
    pub parent_model_id: String,
    pub formatter_config: format::FormatterConfig,
    pub lsp_config: lsp::LspConfigSet,
    pub config_inputs: ConfigInputs,
    pub project_memory: Option<String>,
    pub bus: Arc<EventBus>,
}

/// Provider-audit metadata copied from the parent run so child turns can emit their own
/// `provider_request` records with child-session attribution.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ProviderAuditConfig {
    pub provider: String,
    pub endpoint: String,
}

impl ProviderAuditConfig {
    #[must_use]
    pub(crate) fn new(provider: impl Into<String>, endpoint: impl Into<String>) -> Self {
        Self {
            provider: provider.into(),
            endpoint: endpoint.into(),
        }
    }
}

struct CliTaskRunner {
    config: CliTaskRunnerConfig,
    agents: BTreeMap<String, ActiveAgent>,
    background_tasks: Arc<Mutex<BTreeMap<String, BackgroundTaskEntry>>>,
}

struct BackgroundTaskEntry {
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

    async fn run_background(&self, request: TaskRequest) -> ToolOutput {
        let Some(agent_name) = request.agent.clone() else {
            return ToolOutput::error(
                "task: the 'subagent_type' argument is required when background=true",
            );
        };
        let Some(_agent) = self.agents.get(&agent_name) else {
            return ToolOutput::error(format!("task: subagent `{agent_name}` is not available"));
        };
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
            return ToolOutput::text(render_task_running_output(
                task_id,
                infer_agent_from_title(session.title.as_deref())
                    .as_deref()
                    .unwrap_or("unknown"),
                None,
                None,
            ));
        };
        let agent = infer_agent_from_title(session.title.as_deref())
            .unwrap_or_else(|| "unknown".to_owned());
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
        if let Err(message) = crate::run::validate_model_id(&model_id) {
            return ToolOutput::error(format!("task: {message}"));
        }
        let resolved_model = providers::resolve_model(Some(&model_id));
        let mut turn = Turn::new(
            &model_id,
            render_system_context(&model_id, &agent, self.config.project_memory.clone()),
            prompt,
        )
        .with_history(load_history(&db, &child_session.id).unwrap_or_default())
        .with_compaction(crate::run::compaction_policy(&resolved_model));
        if let Some(temperature) = agent.temperature() {
            turn = turn.with_temperature(temperature);
        }

        let todo_store: Arc<dyn tools::TodoStore> = Arc::new(crate::todo_store::DbTodoStore::new(
            self.config.db_path.clone(),
        ));
        let mut dispatcher = RegistryDispatcher::for_model(
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

        let checkpoint_recorder = match crate::checkpoint::open_store(&self.config.cwd) {
            Ok(Some(store)) => Some(Arc::new(store.begin_turn(child_session.id.clone()))),
            Ok(None) => None,
            Err(err) => {
                eprintln!("cortex run: subagent checkpoint store unavailable: {err}");
                None
            }
        };
        if let Some(recorder) = &checkpoint_recorder {
            dispatcher = dispatcher.with_checkpoints(Arc::new(
                crate::checkpoint::RecorderBridge::new(recorder.clone()),
            ));
        }

        let lsp_hooks = Arc::new(crate::lsp_bridge::LspHooks::new(
            self.config.cwd.clone(),
            self.config.lsp_config.clone(),
        ));
        let mutation_hooks = Arc::new(crate::format_bridge::PostMutationHooks::with_config(
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
            let _ = crate::checkpoint::finalize_turn(recorder);
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
            if let Ok(session) = db.sessions().require(task_id) {
                return Ok((session, true));
            }
        }
        let title = self.child_session_title(agent_name, request.description.as_deref());
        db.sessions()
            .create(Some(&title))
            .map(|session| (session, false))
            .map_err(|err| {
                ToolOutput::error(format!("task: failed to create child session: {err}"))
            })
    }

    fn child_session_title(&self, agent_name: &str, description: Option<&str>) -> String {
        match description {
            Some(description) if !description.trim().is_empty() => format!(
                "{} (subagent:{} parent:{})",
                description.trim(),
                agent_name,
                self.config.parent_session_id
            ),
            _ => format!(
                "subagent:{agent_name} parent:{}",
                self.config.parent_session_id
            ),
        }
    }

    fn child_permissions(
        &self,
        agent: &ActiveAgent,
    ) -> Result<Option<Arc<PermissionEngine>>, RuleError> {
        let builtin = crate::permission::builtin_agent_permission_rules(agent.base_agent());
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
            ruleset.push(rule.clone());
        }

        let has_parent_cap = self.config.parent_cap.is_some();
        if ruleset.is_empty() && !has_parent_cap {
            return Ok(None);
        }
        let mut engine = PermissionEngine::new(ruleset).with_bus(self.config.bus.clone());
        if let Some(parent_cap) = &self.config.parent_cap {
            engine = engine.with_parent_cap(parent_cap.clone());
        }
        Ok(Some(Arc::new(engine)))
    }

    fn provider_for_child(&self, child_audit: Arc<audit::AuditSink>) -> Arc<dyn llm::Provider> {
        let base = match &self.config.provider_audit {
            Some(config) => Arc::new(crate::run::AuditedProvider::new(
                self.config.provider.clone(),
                child_audit.clone(),
                config.provider.clone(),
                config.endpoint.clone(),
            )) as Arc<dyn llm::Provider>,
            None => self.config.provider.clone(),
        };
        self.config
            .plugin_state
            .clone_with_audit(Some(child_audit))
            .wrap_provider(base)
    }
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

fn infer_agent_from_title(title: Option<&str>) -> Option<String> {
    let title = title?;
    let marker = "subagent:";
    let start = title.find(marker)? + marker.len();
    let suffix = &title[start..];
    let agent = suffix
        .split_whitespace()
        .next()
        .unwrap_or("")
        .trim_matches(|ch: char| ch == ')' || ch == ',' || ch == ';');
    (!agent.is_empty()).then(|| agent.to_owned())
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

#[cfg(test)]
mod tests {
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use futures_util::future::BoxFuture;
    use llm::{FinishReason, LlmEvent, LlmRequest, LlmStream, Provider, ToolCall, Usage};
    use permissions::Rule;
    use serde_json::json;

    use super::*;

    static COUNTER: AtomicUsize = AtomicUsize::new(0);

    fn scratch(tag: &str) -> (PathBuf, ConfigInputs) {
        let dir = std::env::temp_dir().join(format!(
            "cortex-subagent-{tag}-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).expect("create scratch");
        let inputs = ConfigInputs {
            home: Some(dir.clone()),
            xdg_config_home: None,
            cwd: dir.clone(),
            env: Vec::new(),
            cli_overrides: BTreeMap::new(),
        };
        (dir, inputs)
    }

    struct ScriptedProvider {
        rounds: Mutex<std::collections::VecDeque<Vec<LlmEvent>>>,
    }

    impl ScriptedProvider {
        fn new(rounds: Vec<Vec<LlmEvent>>) -> Self {
            Self {
                rounds: Mutex::new(rounds.into()),
            }
        }
    }

    impl Provider for ScriptedProvider {
        fn stream<'a>(
            &'a self,
            _request: LlmRequest,
        ) -> BoxFuture<'a, Result<LlmStream, llm::LlmError>> {
            let batch = self
                .rounds
                .lock()
                .expect("rounds")
                .pop_front()
                .unwrap_or_default();
            Box::pin(async move { Ok(Box::pin(futures_util::stream::iter(batch)) as LlmStream) })
        }
    }

    fn parse_task_id(output: &str) -> String {
        output
            .lines()
            .find_map(|line| line.strip_prefix("task_id: ").map(str::to_owned))
            .expect("task output contains task_id")
    }

    #[test]
    fn permitted_catalog_filters_task_targets_through_parent_permissions() {
        let discovery = custom_agents::Discovery::default();
        let engine = PermissionEngine::new(Ruleset::new(vec![
            Rule::new("task", "explore", Action::Allow),
            Rule::new("task", "general", Action::Deny),
            Rule::new("task", "plan", Action::Allow),
        ]));
        let catalog = permitted_catalog(&discovery, Some(&Arc::new(engine)));
        let names: Vec<&str> = catalog
            .targets
            .iter()
            .map(|target| target.name.as_str())
            .collect();
        assert_eq!(names, vec!["plan", "explore"]);
        assert!(catalog.agents.contains_key("plan"));
        assert!(catalog.agents.contains_key("explore"));
        assert!(!catalog.agents.contains_key("general"));
    }

    #[tokio::test]
    async fn subagent_runner_creates_a_distinct_session_and_persists_answer() {
        let (dir, inputs) = scratch("session");
        let db_path = dir.join("cortex.db");
        let db = Database::open(&db_path).expect("db");
        let parent = db
            .sessions()
            .ensure("parent-session", None)
            .expect("parent");

        let catalog = permitted_catalog(&custom_agents::Discovery::default(), None);
        let provider = Arc::new(ScriptedProvider::new(vec![vec![
            LlmEvent::TextDelta {
                text: "child answer".to_owned(),
            },
            LlmEvent::Usage(Usage {
                input_tokens: 1,
                output_tokens: 2,
                total_tokens: 3,
                cache_creation_input_tokens: None,
                cache_read_input_tokens: None,
                reasoning_output_tokens: None,
                cost: Some(0.01),
            }),
            LlmEvent::Done {
                reason: FinishReason::Stop,
            },
        ]]));
        let runner = catalog.runner(CliTaskRunnerConfig {
            provider,
            provider_audit: None,
            plugin_state: PluginState::disabled(),
            audit: Arc::new(audit::AuditSink::new(
                dir.join("audit.jsonl"),
                parent.id.clone(),
            )),
            db_path: db_path.clone(),
            cwd: dir.clone(),
            parent_session_id: parent.id.clone(),
            parent_cap: None,
            parent_model_id: "google/gemini-2.5-flash".to_owned(),
            formatter_config: format::FormatterConfig::default(),
            lsp_config: lsp::LspConfigSet::defaults(),
            config_inputs: inputs,
            project_memory: None,
            bus: Arc::new(EventBus::new()),
        });

        let out = runner
            .run_task(TaskRequest::new("explore", "read the project"))
            .await;
        assert!(!out.is_error, "got: {}", out.model_content());
        assert!(out.model_content().contains("child answer"));
        assert!(out.model_content().contains("parent `parent-session`"));

        let sessions = Database::open(&db_path)
            .expect("db reopen")
            .sessions()
            .list()
            .expect("sessions");
        let child = sessions
            .iter()
            .find(|session| session.id != parent.id)
            .expect("distinct child session");
        assert!(
            child
                .title
                .as_deref()
                .is_some_and(|title| title.contains("subagent:explore"))
        );
        assert!(
            child
                .title
                .as_deref()
                .is_some_and(|title| title.contains("parent:parent-session")),
            "child title should expose durable parent linkage: {:?}",
            child.title
        );
        let messages = Database::open(&db_path)
            .expect("db reopen")
            .messages()
            .list_for_session(&child.id)
            .expect("messages");
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].role, "user");
        assert_eq!(messages[0].content, "read the project");
        assert_eq!(messages[1].role, "assistant");
        assert_eq!(messages[1].content, "child answer");
    }

    #[tokio::test]
    async fn parent_cap_blocks_child_attempt_to_exceed_parent_permissions() {
        let (dir, inputs) = scratch("cap");
        std::fs::write(dir.join("victim.txt"), "alive").expect("victim");
        let db_path = dir.join("cortex.db");
        let db = Database::open(&db_path).expect("db");
        db.sessions()
            .ensure("parent-session", None)
            .expect("parent");

        let catalog = permitted_catalog(&custom_agents::Discovery::default(), None);
        let provider = Arc::new(ScriptedProvider::new(vec![
            vec![
                LlmEvent::ToolCall(ToolCall {
                    id: "call_rm".to_owned(),
                    name: "bash".to_owned(),
                    arguments: json!({"command":"rm victim.txt"}).to_string(),
                }),
                LlmEvent::Done {
                    reason: FinishReason::ToolCalls,
                },
            ],
            vec![
                LlmEvent::TextDelta {
                    text: "rm was blocked".to_owned(),
                },
                LlmEvent::Done {
                    reason: FinishReason::Stop,
                },
            ],
        ]));
        let runner = catalog.runner(CliTaskRunnerConfig {
            provider,
            provider_audit: None,
            plugin_state: PluginState::disabled(),
            audit: Arc::new(audit::AuditSink::new(
                dir.join("audit.jsonl"),
                "parent-session",
            )),
            db_path,
            cwd: dir.clone(),
            parent_session_id: "parent-session".to_owned(),
            parent_cap: Some(Ruleset::new(vec![
                Rule::new("bash", "*", Action::Allow),
                Rule::new("bash", "rm *", Action::Deny),
            ])),
            parent_model_id: "google/gemini-2.5-flash".to_owned(),
            formatter_config: format::FormatterConfig::default(),
            lsp_config: lsp::LspConfigSet::defaults(),
            config_inputs: inputs,
            project_memory: None,
            bus: Arc::new(EventBus::new()),
        });

        let out = runner
            .run_task(TaskRequest::new("general", "remove victim.txt"))
            .await;
        assert!(!out.is_error, "got: {}", out.model_content());
        assert!(
            out.model_content().contains("tools: bash"),
            "child tool activity is reported separately: {}",
            out.model_content()
        );
        assert_eq!(
            std::fs::read_to_string(dir.join("victim.txt")).expect("victim survives"),
            "alive"
        );
        let records = audit::read_jsonl(&dir.join("audit.jsonl")).expect("audit records");
        assert!(
            records.iter().any(|record| {
                record["type"] == "permission_decision"
                    && record["session"].as_str() != Some("parent-session")
                    && record["parent_session"] == "parent-session"
                    && record["tool"] == "bash"
                    && record["request"] == "rm victim.txt"
                    && record["outcome"] == "denied"
            }),
            "{records:#?}"
        );
        assert!(
            !records.iter().any(|record| {
                record["type"] == "shell_execution" && record["command"] == "rm victim.txt"
            }),
            "{records:#?}"
        );
    }

    #[tokio::test]
    async fn child_provider_requests_are_audited_under_the_child_session() {
        let (dir, inputs) = scratch("child-provider-audit");
        let db_path = dir.join("cortex.db");
        let db = Database::open(&db_path).expect("db");
        db.sessions()
            .ensure("parent-session", None)
            .expect("parent");

        let catalog = permitted_catalog(&custom_agents::Discovery::default(), None);
        let provider = Arc::new(ScriptedProvider::new(vec![vec![
            LlmEvent::TextDelta {
                text: "child provider call".to_owned(),
            },
            LlmEvent::Done {
                reason: FinishReason::Stop,
            },
        ]]));
        let runner = catalog.runner(CliTaskRunnerConfig {
            provider,
            provider_audit: Some(ProviderAuditConfig::new(
                "openrouter",
                "https://openrouter.ai/api/v1/chat/completions",
            )),
            plugin_state: PluginState::disabled(),
            audit: Arc::new(audit::AuditSink::new(
                dir.join("audit.jsonl"),
                "parent-session",
            )),
            db_path,
            cwd: dir.clone(),
            parent_session_id: "parent-session".to_owned(),
            parent_cap: None,
            parent_model_id: "google/gemini-2.5-flash".to_owned(),
            formatter_config: format::FormatterConfig::default(),
            lsp_config: lsp::LspConfigSet::defaults(),
            config_inputs: inputs,
            project_memory: None,
            bus: Arc::new(EventBus::new()),
        });

        let out = runner
            .run_task(TaskRequest::new("explore", "run a child turn"))
            .await;
        assert!(!out.is_error, "got: {}", out.model_content());
        let output = out.model_content().to_owned();
        let child_session = output
            .split_once("completed in session `")
            .and_then(|(_, rest)| rest.split_once('`').map(|(id, _)| id))
            .expect("task output exposes child session id");

        let records = audit::read_jsonl(&dir.join("audit.jsonl")).expect("audit records");
        assert!(
            records.iter().any(|record| {
                record["type"] == "network_call"
                    && record["kind"] == "provider_request"
                    && record["session"] == child_session
                    && record["parent_session"] == "parent-session"
                    && record["parent_session_id"] == "parent-session"
            }),
            "{records:#?}"
        );
        assert!(
            !records.iter().any(|record| {
                record["type"] == "network_call"
                    && record["kind"] == "provider_request"
                    && record["session"] == "parent-session"
            }),
            "{records:#?}"
        );
    }

    #[tokio::test]
    async fn task_id_resume_reuses_the_same_child_session() {
        let (dir, inputs) = scratch("resume");
        let db_path = dir.join("cortex.db");
        let db = Database::open(&db_path).expect("db");
        db.sessions()
            .ensure("parent-session", None)
            .expect("parent");

        let catalog = permitted_catalog(&custom_agents::Discovery::default(), None);
        let provider = Arc::new(ScriptedProvider::new(vec![
            vec![
                LlmEvent::TextDelta {
                    text: "first child answer".to_owned(),
                },
                LlmEvent::Done {
                    reason: FinishReason::Stop,
                },
            ],
            vec![
                LlmEvent::TextDelta {
                    text: "second child answer".to_owned(),
                },
                LlmEvent::Done {
                    reason: FinishReason::Stop,
                },
            ],
        ]));
        let runner = catalog.runner(CliTaskRunnerConfig {
            provider,
            provider_audit: None,
            plugin_state: PluginState::disabled(),
            audit: Arc::new(audit::AuditSink::new(
                dir.join("audit.jsonl"),
                "parent-session",
            )),
            db_path: db_path.clone(),
            cwd: dir.clone(),
            parent_session_id: "parent-session".to_owned(),
            parent_cap: None,
            parent_model_id: "google/gemini-2.5-flash".to_owned(),
            formatter_config: format::FormatterConfig::default(),
            lsp_config: lsp::LspConfigSet::defaults(),
            config_inputs: inputs,
            project_memory: None,
            bus: Arc::new(EventBus::new()),
        });

        let first = runner
            .run_task(TaskRequest::new("explore", "first prompt"))
            .await;
        assert!(!first.is_error, "got: {}", first.model_content());
        let task_id = parse_task_id(&first.model_content());
        assert!(
            first.model_content().contains("resumed: false"),
            "first run should create a new child session: {}",
            first.model_content()
        );

        let second = runner
            .run_task(TaskRequest {
                agent: Some("explore".to_owned()),
                prompt: Some("second prompt".to_owned()),
                description: Some("resume run".to_owned()),
                task_id: Some(task_id.clone()),
                command: Some("scan --resume".to_owned()),
                background: false,
            })
            .await;
        assert!(!second.is_error, "got: {}", second.model_content());
        assert!(
            second
                .model_content()
                .contains(&format!("task_id: {task_id}")),
            "resume should target the same task_id: {}",
            second.model_content()
        );
        assert!(
            second.model_content().contains("resumed: true"),
            "resume output should mark resumed=true: {}",
            second.model_content()
        );
        let messages = Database::open(&db_path)
            .expect("db reopen")
            .messages()
            .list_for_session(&task_id)
            .expect("messages");
        assert!(
            messages.iter().any(|message| {
                message.role == "user" && message.content.contains("first prompt")
            }),
            "{messages:#?}"
        );
        assert!(
            messages.iter().any(|message| {
                message.role == "user" && message.content.contains("second prompt")
            }),
            "{messages:#?}"
        );
    }

    #[tokio::test]
    async fn background_task_reports_running_and_rejoin_returns_completion() {
        let (dir, inputs) = scratch("background-rejoin");
        let db_path = dir.join("cortex.db");
        let db = Database::open(&db_path).expect("db");
        db.sessions()
            .ensure("parent-session", None)
            .expect("parent");

        let catalog = permitted_catalog(&custom_agents::Discovery::default(), None);
        let provider = Arc::new(ScriptedProvider::new(vec![vec![
            LlmEvent::TextDelta {
                text: "background child done".to_owned(),
            },
            LlmEvent::Done {
                reason: FinishReason::Stop,
            },
        ]]));
        let runner = catalog.runner(CliTaskRunnerConfig {
            provider,
            provider_audit: None,
            plugin_state: PluginState::disabled(),
            audit: Arc::new(audit::AuditSink::new(
                dir.join("audit.jsonl"),
                "parent-session",
            )),
            db_path,
            cwd: dir.clone(),
            parent_session_id: "parent-session".to_owned(),
            parent_cap: None,
            parent_model_id: "google/gemini-2.5-flash".to_owned(),
            formatter_config: format::FormatterConfig::default(),
            lsp_config: lsp::LspConfigSet::defaults(),
            config_inputs: inputs,
            project_memory: None,
            bus: Arc::new(EventBus::new()),
        });

        let started = runner
            .run_task(TaskRequest {
                agent: Some("explore".to_owned()),
                prompt: Some("run in background".to_owned()),
                description: Some("background test".to_owned()),
                task_id: None,
                command: Some("scan repo".to_owned()),
                background: true,
            })
            .await;
        assert!(!started.is_error, "got: {}", started.model_content());
        assert!(
            started.model_content().contains("state: running"),
            "background launch should return running state: {}",
            started.model_content()
        );
        let task_id = parse_task_id(&started.model_content());

        let mut rejoined = ToolOutput::error("not started");
        for _ in 0..40 {
            rejoined = runner
                .run_task(TaskRequest {
                    agent: None,
                    prompt: None,
                    description: None,
                    task_id: Some(task_id.clone()),
                    command: None,
                    background: false,
                })
                .await;
            if rejoined.model_content().contains("state: completed") {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        assert!(
            rejoined.model_content().contains("state: completed"),
            "rejoin should eventually return completion: {}",
            rejoined.model_content()
        );
        assert!(
            rejoined.model_content().contains("background child done"),
            "completed rejoin should include the child answer: {}",
            rejoined.model_content()
        );
    }

    #[tokio::test]
    async fn subagent_background_process_is_reaped_after_child_turn() {
        let (dir, inputs) = scratch("background");
        let db_path = dir.join("cortex.db");
        let db = Database::open(&db_path).expect("db");
        db.sessions()
            .ensure("parent-session", None)
            .expect("parent");

        let catalog = permitted_catalog(&custom_agents::Discovery::default(), None);
        let provider = Arc::new(ScriptedProvider::new(vec![
            vec![
                LlmEvent::ToolCall(ToolCall {
                    id: "call_bg".to_owned(),
                    name: "bash".to_owned(),
                    arguments: json!({
                        "command": "echo $$ > bg.pid; sleep 30",
                        "background": true
                    })
                    .to_string(),
                }),
                LlmEvent::Done {
                    reason: FinishReason::ToolCalls,
                },
            ],
            vec![
                LlmEvent::TextDelta {
                    text: "background started".to_owned(),
                },
                LlmEvent::Done {
                    reason: FinishReason::Stop,
                },
            ],
        ]));
        let runner = catalog.runner(CliTaskRunnerConfig {
            provider,
            provider_audit: None,
            plugin_state: PluginState::disabled(),
            audit: Arc::new(audit::AuditSink::new(
                dir.join("audit.jsonl"),
                "parent-session",
            )),
            db_path,
            cwd: dir.clone(),
            parent_session_id: "parent-session".to_owned(),
            parent_cap: None,
            parent_model_id: "google/gemini-2.5-flash".to_owned(),
            formatter_config: format::FormatterConfig::default(),
            lsp_config: lsp::LspConfigSet::defaults(),
            config_inputs: inputs,
            project_memory: None,
            bus: Arc::new(EventBus::new()),
        });

        let out = runner
            .run_task(TaskRequest::new("general", "start a background sleep"))
            .await;
        assert!(!out.is_error, "got: {}", out.model_content());
        let text = out.model_content();
        let pid = text
            .lines()
            .find_map(|line| line.trim().strip_prefix("pid: "))
            .expect("task output includes the child background pid")
            .parse::<u32>()
            .expect("numeric pid");
        for _ in 0..20 {
            if !std::path::Path::new(&format!("/proc/{pid}")).exists() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(
            !std::path::Path::new(&format!("/proc/{pid}")).exists(),
            "subagent background process {pid} should have been reaped"
        );
    }
}
