//! Headless `task` subagent execution.
//!
//! The `tools` crate owns the model-facing `task` tool, while the CLI owns the real durable child
//! turn: session creation, prompt/persona assembly, permission narrowing, child tool materializing,
//! checkpoint/formatter/LSP wiring, and exact-PID cleanup of any processes the child tools spawn.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use db::Database;
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
pub(crate) struct CliTaskRunnerConfig {
    pub provider: Arc<dyn llm::Provider>,
    pub audit: Arc<audit::AuditSink>,
    pub db_path: PathBuf,
    pub cwd: PathBuf,
    pub parent_session_id: String,
    pub parent_cap: Option<Ruleset>,
    pub parent_model_id: String,
    pub formatter_config: format::FormatterConfig,
    pub config_inputs: ConfigInputs,
    pub project_memory: Option<String>,
    pub bus: Arc<EventBus>,
}

struct CliTaskRunner {
    config: CliTaskRunnerConfig,
    agents: BTreeMap<String, ActiveAgent>,
}

impl TaskRunner for CliTaskRunner {
    fn run_task<'a>(&'a self, request: TaskRequest) -> BoxFuture<'a, ToolOutput> {
        Box::pin(async move { self.run(request).await })
    }
}

impl CliTaskRunner {
    async fn run(&self, request: TaskRequest) -> ToolOutput {
        let Some(agent) = self.agents.get(&request.agent).cloned() else {
            return ToolOutput::error(format!(
                "task: subagent `{}` is not available",
                request.agent
            ));
        };

        let db = match Database::open(&self.config.db_path) {
            Ok(db) => db,
            Err(err) => return ToolOutput::error(format!("task: failed to open database: {err}")),
        };
        let title = format!(
            "subagent:{} parent:{}",
            request.agent, self.config.parent_session_id
        );
        let child_session = match db.sessions().create(Some(&title)) {
            Ok(session) => session,
            Err(err) => {
                return ToolOutput::error(format!("task: failed to create child session: {err}"));
            }
        };
        let child_audit = Arc::new(self.config.audit.child_session(child_session.id.clone()));

        if let Err(err) = db
            .messages()
            .create(&child_session.id, "user", &request.prompt)
        {
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
            request.prompt,
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

        let lsp_hooks = Arc::new(crate::lsp_bridge::LspHooks::new(self.config.cwd.clone()));
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
            .run_with_tools(self.config.provider.as_ref(), turn, &dispatcher, |event| {
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
            crate::checkpoint::finalize_turn(recorder);
        }

        persist_usage(&db, &child_session.id, &outcome);
        if let Some(err) = &outcome.error {
            return ToolOutput::error(format!(
                "Subagent `{}` failed in session `{}`: {}",
                request.agent, child_session.id, err
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
        ToolOutput::text(format!(
            "Subagent `{}` completed in session `{}` (parent `{}`; tools: {}).\n\n{}{}",
            request.agent,
            child_session.id,
            self.config.parent_session_id,
            tool_summary,
            outcome.answer(),
            tool_detail_section
        ))
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
                cost: Some(0.01),
            }),
            LlmEvent::Done {
                reason: FinishReason::Stop,
            },
        ]]));
        let runner = catalog.runner(CliTaskRunnerConfig {
            provider,
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
