use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use db::Database;
use events::EventBus;
use futures_util::future::BoxFuture;
use llm::{FinishReason, LlmEvent, LlmRequest, LlmStream, Provider, ToolCall, Usage};
use permissions::{
    Action, Decision, DecisionOverride, PermissionEngine, PermissionRequest, Rule, Ruleset,
};
use serde_json::json;
use settings::ConfigInputs;
use std::sync::Arc;
use tools::{TaskRequest, TaskRunner, ToolOutput};

use crate::custom_agents;
use crate::provider::ProviderAuditConfig;
use crate::runner::CliTaskRunnerConfig;
use crate::{SubagentCatalog, permitted_catalog};

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

/// Build a config with no provider-audit and no plugin wrapper (the shared default), matching the
/// TUI surface and the CLI surface without a plugin provider substitution.
fn config(dir: &std::path::Path, db_path: PathBuf, inputs: ConfigInputs) -> ConfigBuilder {
    ConfigBuilder {
        provider: None,
        provider_audit: None,
        parent_cap: None,
        parent_permission_override: None,
        audit_path: dir.join("audit.jsonl"),
        parent_session_id: "parent-session".to_owned(),
        db_path,
        cwd: dir.to_path_buf(),
        inputs,
    }
}

struct ConfigBuilder {
    provider: Option<Arc<dyn Provider>>,
    provider_audit: Option<ProviderAuditConfig>,
    parent_cap: Option<Ruleset>,
    parent_permission_override: Option<Arc<dyn DecisionOverride>>,
    audit_path: PathBuf,
    parent_session_id: String,
    db_path: PathBuf,
    cwd: PathBuf,
    inputs: ConfigInputs,
}

impl ConfigBuilder {
    fn provider(mut self, provider: Arc<dyn Provider>) -> Self {
        self.provider = Some(provider);
        self
    }

    fn provider_audit(mut self, audit: ProviderAuditConfig) -> Self {
        self.provider_audit = Some(audit);
        self
    }

    fn parent_cap(mut self, cap: Ruleset) -> Self {
        self.parent_cap = Some(cap);
        self
    }

    fn parent_permission_override(mut self, override_hook: Arc<dyn DecisionOverride>) -> Self {
        self.parent_permission_override = Some(override_hook);
        self
    }

    fn build(self) -> CliTaskRunnerConfig {
        CliTaskRunnerConfig {
            provider: self.provider.expect("provider set"),
            provider_audit: self.provider_audit,
            plugin_provider: None,
            audit: Arc::new(audit::AuditSink::new(
                self.audit_path,
                self.parent_session_id.clone(),
            )),
            db_path: self.db_path,
            cwd: self.cwd,
            parent_session_id: self.parent_session_id,
            parent_cap: self.parent_cap,
            parent_permission_override: self.parent_permission_override,
            parent_model_id: "google/gemini-2.5-flash".to_owned(),
            formatter_config: format::FormatterConfig::default(),
            lsp_config: lsp::LspConfigSet::defaults(),
            config_inputs: self.inputs,
            project_memory: memory::ProjectMemory::default(),
            bus: Arc::new(EventBus::new()),
        }
    }
}

fn runner(catalog: &SubagentCatalog, config: CliTaskRunnerConfig) -> Arc<crate::CliTaskRunner> {
    catalog.runner(config)
}

struct ScriptedProvider {
    rounds: Mutex<std::collections::VecDeque<Vec<LlmEvent>>>,
    calls: AtomicUsize,
    requests: Mutex<Vec<LlmRequest>>,
}

impl ScriptedProvider {
    fn new(rounds: Vec<Vec<LlmEvent>>) -> Self {
        Self {
            rounds: Mutex::new(rounds.into()),
            calls: AtomicUsize::new(0),
            requests: Mutex::new(Vec::new()),
        }
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::Relaxed)
    }

    fn requests(&self) -> Vec<LlmRequest> {
        self.requests.lock().expect("requests lock").clone()
    }
}

impl Provider for ScriptedProvider {
    fn stream<'a>(
        &'a self,
        request: LlmRequest,
    ) -> BoxFuture<'a, Result<LlmStream, llm::LlmError>> {
        self.requests.lock().expect("requests lock").push(request);
        self.calls.fetch_add(1, Ordering::Relaxed);
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
    let run = runner(
        &catalog,
        config(&dir, db_path.clone(), inputs)
            .provider(provider)
            .build(),
    );

    let out = run
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
    assert_eq!(
        child.parent_id.as_deref(),
        Some(parent.id.as_str()),
        "a task child must carry a structured durable parent link"
    );
    assert!(
        child.title.is_none(),
        "task children without a description have no synthetic title marker"
    );
    assert_eq!(
        child.subagent_type.as_deref(),
        Some("explore"),
        "a task child persists its normalized persona separately from title"
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
async fn subagent_refreshes_nested_memory_before_same_turn_continuation() {
    let (dir, inputs) = scratch("dynamic-memory");
    let nested = dir.join("nested");
    std::fs::create_dir_all(&nested).expect("nested");
    std::fs::write(dir.join("AGENTS.md"), "SUBAGENT-ROOT-DYNAMIC-MEMORY").expect("root memory");
    std::fs::write(nested.join("AGENTS.md"), "SUBAGENT-NESTED-DYNAMIC-MEMORY")
        .expect("nested memory");
    std::fs::write(nested.join("target.txt"), "read me").expect("target");
    let db_path = dir.join("cortex.db");
    Database::open(&db_path)
        .expect("db")
        .sessions()
        .ensure("parent-session", None)
        .expect("parent");
    let initial_memory = memory::discover(&inputs);
    let provider = Arc::new(ScriptedProvider::new(vec![
        vec![
            LlmEvent::ToolCall(ToolCall {
                id: "call_nested_read".to_owned(),
                name: "read".to_owned(),
                arguments: json!({ "path": "nested/target.txt" }).to_string(),
            }),
            LlmEvent::Done {
                reason: FinishReason::ToolCalls,
            },
        ],
        vec![
            LlmEvent::TextDelta {
                text: "nested memory is active".to_owned(),
            },
            LlmEvent::Done {
                reason: FinishReason::Stop,
            },
        ],
    ]));
    let catalog = permitted_catalog(&custom_agents::Discovery::default(), None);
    let mut run_config = config(&dir, db_path, inputs)
        .provider(provider.clone())
        .build();
    run_config.project_memory = initial_memory;
    let run = runner(&catalog, run_config);

    let output = run
        .run_task(TaskRequest::new("general", "read nested/target.txt"))
        .await;

    assert!(!output.is_error, "{}", output.model_content());
    let requests = provider.requests();
    assert_eq!(requests.len(), 2, "tool call should cause one continuation");
    let systems: Vec<&str> = requests
        .iter()
        .map(|request| {
            request
                .messages
                .iter()
                .find(|message| message.role == llm::Role::System)
                .map(|message| message.content.as_str())
                .expect("system message")
        })
        .collect();
    assert!(systems[0].contains("SUBAGENT-ROOT-DYNAMIC-MEMORY"));
    assert!(!systems[0].contains("SUBAGENT-NESTED-DYNAMIC-MEMORY"));
    let root = systems[1]
        .find("SUBAGENT-ROOT-DYNAMIC-MEMORY")
        .expect("root memory");
    let nested = systems[1]
        .find("SUBAGENT-NESTED-DYNAMIC-MEMORY")
        .expect("nested memory");
    assert!(root < nested, "memory sources must stay root-to-leaf");
}

#[tokio::test]
async fn subagent_refreshes_nested_memory_after_foreground_bash_workdir() {
    let (dir, inputs) = scratch("dynamic-workdir-memory");
    let nested = dir.join("nested");
    std::fs::create_dir_all(&nested).expect("nested");
    std::fs::write(dir.join("AGENTS.md"), "SUBAGENT-WORKDIR-ROOT-MEMORY").expect("root memory");
    std::fs::write(nested.join("AGENTS.md"), "SUBAGENT-WORKDIR-NESTED-MEMORY")
        .expect("nested memory");
    let db_path = dir.join("cortex.db");
    Database::open(&db_path)
        .expect("db")
        .sessions()
        .ensure("parent-session", None)
        .expect("parent");
    let initial_memory = memory::discover(&inputs);
    let provider = Arc::new(ScriptedProvider::new(vec![
        vec![
            LlmEvent::ToolCall(ToolCall {
                id: "call_nested_workdir".to_owned(),
                name: "bash".to_owned(),
                arguments: json!({ "command": "pwd", "workdir": "nested" }).to_string(),
            }),
            LlmEvent::Done {
                reason: FinishReason::ToolCalls,
            },
        ],
        vec![
            LlmEvent::TextDelta {
                text: "nested memory is active".to_owned(),
            },
            LlmEvent::Done {
                reason: FinishReason::Stop,
            },
        ],
    ]));
    let catalog = permitted_catalog(&custom_agents::Discovery::default(), None);
    let mut run_config = config(&dir, db_path, inputs)
        .provider(provider.clone())
        .build();
    run_config.project_memory = initial_memory;
    let run = runner(&catalog, run_config);

    let output = run
        .run_task(TaskRequest::new("general", "run pwd in nested"))
        .await;

    assert!(!output.is_error, "{}", output.model_content());
    let requests = provider.requests();
    assert_eq!(requests.len(), 2, "tool call should cause one continuation");
    let systems: Vec<&str> = requests
        .iter()
        .map(|request| {
            request
                .messages
                .iter()
                .find(|message| message.role == llm::Role::System)
                .map(|message| message.content.as_str())
                .expect("system message")
        })
        .collect();
    assert!(systems[0].contains("SUBAGENT-WORKDIR-ROOT-MEMORY"));
    assert!(!systems[0].contains("SUBAGENT-WORKDIR-NESTED-MEMORY"));
    let root = systems[1]
        .find("SUBAGENT-WORKDIR-ROOT-MEMORY")
        .expect("root memory");
    let nested = systems[1]
        .find("SUBAGENT-WORKDIR-NESTED-MEMORY")
        .expect("nested memory");
    assert!(root < nested, "memory sources must stay root-to-leaf");
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
    let run = runner(
        &catalog,
        config(&dir, db_path, inputs)
            .provider(provider)
            .parent_cap(Ruleset::new(vec![
                Rule::new("bash", "*", Action::Allow),
                Rule::new("bash", "rm *", Action::Deny),
            ]))
            .build(),
    );

    let out = run
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
async fn parent_dynamic_deny_override_also_narrows_child_permissions() {
    struct ParentWriteDeny;

    impl DecisionOverride for ParentWriteDeny {
        fn override_decision(
            &self,
            request: &PermissionRequest,
            _proposed: Action,
        ) -> Option<Decision> {
            (request.tool == "write").then_some(Decision::Deny)
        }
    }

    let (dir, inputs) = scratch("dynamic-parent-deny");
    let db_path = dir.join("cortex.db");
    Database::open(&db_path)
        .expect("db")
        .sessions()
        .ensure("parent-session", None)
        .expect("parent");

    let catalog = permitted_catalog(&custom_agents::Discovery::default(), None);
    let provider = Arc::new(ScriptedProvider::new(vec![
        vec![
            LlmEvent::ToolCall(ToolCall {
                id: "call_write".to_owned(),
                name: "write".to_owned(),
                arguments: json!({
                    "path": "plugin-denied.txt",
                    "content": "must stay absent"
                })
                .to_string(),
            }),
            LlmEvent::Done {
                reason: FinishReason::ToolCalls,
            },
        ],
        vec![
            LlmEvent::TextDelta {
                text: "the write was denied by the parent policy".to_owned(),
            },
            LlmEvent::Done {
                reason: FinishReason::Stop,
            },
        ],
    ]));
    let run = runner(
        &catalog,
        config(&dir, db_path, inputs)
            .provider(provider)
            .parent_cap(Ruleset::new(vec![Rule::new("write", "*", Action::Allow)]))
            .parent_permission_override(Arc::new(ParentWriteDeny))
            .build(),
    );

    let out = run
        .run_task(TaskRequest::new("general", "write plugin-denied.txt"))
        .await;

    assert!(!out.is_error, "got: {}", out.model_content());
    assert!(
        out.model_content().contains("write: permission denied"),
        "the child tool result must preserve the parent's dynamic denial: {}",
        out.model_content()
    );
    assert!(
        !dir.join("plugin-denied.txt").exists(),
        "a dynamically denied parent write must stay denied in the child"
    );
}

#[tokio::test]
async fn explore_subagent_uses_its_read_only_persona_and_cannot_write() {
    let (dir, inputs) = scratch("explore-read-only");
    let db_path = dir.join("cortex.db");
    Database::open(&db_path)
        .expect("db")
        .sessions()
        .ensure("parent-session", None)
        .expect("parent");

    let catalog = permitted_catalog(&custom_agents::Discovery::default(), None);
    let provider = Arc::new(ScriptedProvider::new(vec![
        vec![
            LlmEvent::ToolCall(ToolCall {
                id: "call_write".to_owned(),
                name: "write".to_owned(),
                arguments: json!({
                    "path": "forbidden.txt",
                    "content": "this must not be written"
                })
                .to_string(),
            }),
            LlmEvent::Done {
                reason: FinishReason::ToolCalls,
            },
        ],
        vec![
            LlmEvent::TextDelta {
                text: "I inspected the project without changing it.".to_owned(),
            },
            LlmEvent::Done {
                reason: FinishReason::Stop,
            },
        ],
    ]));
    let run = runner(
        &catalog,
        config(&dir, db_path, inputs).provider(provider).build(),
    );

    let out = run
        .run_task(TaskRequest::new(
            "explore",
            "Try to write forbidden.txt, then report what happened.",
        ))
        .await;

    assert!(!out.is_error, "got: {}", out.model_content());
    assert!(
        out.model_content().contains("write: permission denied"),
        "the child tool result must show the explore restriction: {}",
        out.model_content()
    );
    assert!(
        !dir.join("forbidden.txt").exists(),
        "the explore child must not mutate even when its prompt requests a write"
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
    let run = runner(
        &catalog,
        config(&dir, db_path, inputs)
            .provider(provider)
            .provider_audit(ProviderAuditConfig::new(
                "openrouter",
                "https://openrouter.ai/api/v1/chat/completions",
            ))
            .build(),
    );

    let out = run
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
    let run = runner(
        &catalog,
        config(&dir, db_path.clone(), inputs)
            .provider(provider)
            .build(),
    );

    let first = run
        .run_task(TaskRequest::new("explore", "first prompt"))
        .await;
    assert!(!first.is_error, "got: {}", first.model_content());
    let task_id = parse_task_id(&first.model_content());
    assert!(
        first.model_content().contains("resumed: false"),
        "first run should create a new child session: {}",
        first.model_content()
    );

    let second = run
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
        messages
            .iter()
            .any(|message| { message.role == "user" && message.content.contains("first prompt") }),
        "{messages:#?}"
    );
    assert!(
        messages
            .iter()
            .any(|message| { message.role == "user" && message.content.contains("second prompt") }),
        "{messages:#?}"
    );
}

#[tokio::test]
async fn task_rejoin_rejects_a_child_owned_by_a_different_parent() {
    let (dir, inputs) = scratch("foreign-rejoin");
    let db_path = dir.join("cortex.db");
    let db = Database::open(&db_path).expect("db");
    db.sessions()
        .ensure("parent-session", None)
        .expect("parent");
    db.sessions()
        .ensure("other-parent", None)
        .expect("other parent");

    let catalog = permitted_catalog(&custom_agents::Discovery::default(), None);
    let provider = Arc::new(ScriptedProvider::new(vec![vec![
        LlmEvent::TextDelta {
            text: "private child answer".to_owned(),
        },
        LlmEvent::Done {
            reason: FinishReason::Stop,
        },
    ]]));
    let owner = runner(
        &catalog,
        config(&dir, db_path.clone(), inputs.clone())
            .provider(provider.clone())
            .build(),
    );
    let created = owner
        .run_task(TaskRequest::new("explore", "produce a private result"))
        .await;
    assert!(!created.is_error, "got: {}", created.model_content());
    let task_id = parse_task_id(&created.model_content());
    let messages_before = Database::open(&db_path)
        .expect("db")
        .messages()
        .list_for_session(&task_id)
        .expect("messages")
        .len();

    let mut foreign_config = config(&dir, db_path.clone(), inputs)
        .provider(Arc::new(ScriptedProvider::new(Vec::new())))
        .build();
    foreign_config.parent_session_id = "other-parent".to_owned();
    let foreign = runner(&catalog, foreign_config);
    let rejoined = foreign
        .run_task(TaskRequest {
            agent: None,
            prompt: None,
            description: None,
            task_id: Some(task_id.clone()),
            command: None,
            background: false,
        })
        .await;

    assert!(rejoined.is_error, "got: {}", rejoined.model_content());
    assert!(
        rejoined
            .model_content()
            .contains("does not belong to parent session"),
        "a task id must not disclose another parent's child result: {}",
        rejoined.model_content()
    );
    let messages_after = Database::open(&db_path)
        .expect("db")
        .messages()
        .list_for_session(&task_id)
        .expect("messages")
        .len();
    assert_eq!(
        messages_after, messages_before,
        "a rejected foreign rejoin must not alter the child transcript"
    );
    assert_eq!(
        provider.calls(),
        1,
        "foreign ownership rejection must happen before another provider request"
    );
}

#[tokio::test]
async fn task_resume_cannot_change_the_child_subagent_persona() {
    let (dir, inputs) = scratch("persona-resume");
    let db_path = dir.join("cortex.db");
    Database::open(&db_path)
        .expect("db")
        .sessions()
        .ensure("parent-session", None)
        .expect("parent");

    let catalog = permitted_catalog(&custom_agents::Discovery::default(), None);
    let provider = Arc::new(ScriptedProvider::new(vec![vec![
        LlmEvent::TextDelta {
            text: "explore child answer".to_owned(),
        },
        LlmEvent::Done {
            reason: FinishReason::Stop,
        },
    ]]));
    let run = runner(
        &catalog,
        config(&dir, db_path.clone(), inputs)
            .provider(provider.clone())
            .build(),
    );
    let created = run
        .run_task(TaskRequest::new("explore", "inspect without changes"))
        .await;
    assert!(!created.is_error, "got: {}", created.model_content());
    let task_id = parse_task_id(&created.model_content());
    Database::open(&db_path)
        .expect("db")
        .sessions()
        .set_title(
            &task_id,
            Some("subagent:general parent:attacker-controlled-display-text"),
        )
        .expect("rename child to a spoofed display title");
    let messages_before = Database::open(&db_path)
        .expect("db")
        .messages()
        .list_for_session(&task_id)
        .expect("messages")
        .len();

    let resumed = run
        .run_task(TaskRequest {
            agent: Some("general".to_owned()),
            prompt: Some("now mutate the project".to_owned()),
            description: None,
            task_id: Some(task_id.clone()),
            command: None,
            background: false,
        })
        .await;

    assert!(resumed.is_error, "got: {}", resumed.model_content());
    assert!(
        resumed
            .model_content()
            .contains("runs as subagent `explore`, not `general`"),
        "a resume must retain the existing child persona: {}",
        resumed.model_content()
    );
    let messages_after = Database::open(&db_path)
        .expect("db")
        .messages()
        .list_for_session(&task_id)
        .expect("messages")
        .len();
    assert_eq!(
        messages_after, messages_before,
        "a rejected persona change must not append to the child transcript"
    );
    assert_eq!(
        provider.calls(),
        1,
        "persona rejection must happen before the child provider executes"
    );
}

#[tokio::test]
async fn title_only_legacy_children_fail_closed_for_rejoin_and_resume() {
    let (dir, inputs) = scratch("legacy-title-only");
    let db_path = dir.join("cortex.db");
    let db = Database::open(&db_path).expect("db");
    db.sessions()
        .ensure("parent-session", None)
        .expect("parent");
    let legacy = db
        .sessions()
        .create_child(
            "parent-session",
            Some("subagent:explore parent:parent-session"),
        )
        .expect("legacy title-only child");
    db.messages()
        .create(&legacy.id, "assistant", "must not be disclosed")
        .expect("legacy answer");

    let catalog = permitted_catalog(&custom_agents::Discovery::default(), None);
    let provider = Arc::new(ScriptedProvider::new(Vec::new()));
    let run = runner(
        &catalog,
        config(&dir, db_path.clone(), inputs)
            .provider(provider.clone())
            .build(),
    );

    let rejoin = run
        .run_task(TaskRequest {
            agent: None,
            prompt: None,
            description: None,
            task_id: Some(legacy.id.clone()),
            command: None,
            background: false,
        })
        .await;
    assert!(rejoin.is_error, "got: {}", rejoin.model_content());
    assert!(
        rejoin.model_content().contains("not a valid subagent task"),
        "title-only legacy rows must fail closed: {}",
        rejoin.model_content()
    );

    let resume = run
        .run_task(TaskRequest {
            agent: Some("explore".to_owned()),
            prompt: Some("try a resumed turn".to_owned()),
            description: None,
            task_id: Some(legacy.id.clone()),
            command: None,
            background: false,
        })
        .await;
    assert!(resume.is_error, "got: {}", resume.model_content());
    assert!(
        resume.model_content().contains("not a valid subagent task"),
        "resume also rejects legacy title-only rows: {}",
        resume.model_content()
    );
    assert_eq!(
        provider.calls(),
        0,
        "legacy authorization fails before provider execution"
    );
    assert_eq!(
        Database::open(&db_path)
            .expect("db")
            .messages()
            .list_for_session(&legacy.id)
            .expect("legacy messages")
            .len(),
        1,
        "rejected legacy rows do not receive a prompt or assistant message"
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
    let run = runner(
        &catalog,
        config(&dir, db_path, inputs).provider(provider).build(),
    );

    let started = run
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
        rejoined = run
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
async fn drain_background_persists_child_transcript_for_restart_rejoin() {
    // A background `task` child turn is a spawned future on the parent's current-thread runtime;
    // it only makes progress while the runtime drives it. The parent shutdown drain must run it
    // to completion so the transcript persists durably and a SEPARATE process (fresh runner +
    // fresh DB handle, no in-memory background map) can still rejoin it by id after a restart.
    let (dir, inputs) = scratch("drain-restart");
    let db_path = dir.join("cortex.db");
    let db = Database::open(&db_path).expect("db");
    db.sessions()
        .ensure("parent-session", None)
        .expect("parent");

    let catalog = permitted_catalog(&custom_agents::Discovery::default(), None);
    let provider = Arc::new(ScriptedProvider::new(vec![vec![
        LlmEvent::TextDelta {
            text: "drained child answer".to_owned(),
        },
        LlmEvent::Done {
            reason: FinishReason::Stop,
        },
    ]]));
    let built = config(&dir, db_path.clone(), inputs)
        .provider(provider)
        .build();
    let run = runner(&catalog, built.clone());

    let started = run
        .run_task(TaskRequest {
            agent: Some("explore".to_owned()),
            prompt: Some("durable background work".to_owned()),
            description: Some("restart test".to_owned()),
            task_id: None,
            command: None,
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

    // Parent process shutdown: drive the in-flight background child to completion.
    run.drain_background().await;

    // Restart: a brand-new DB handle must already see the full child transcript.
    let messages = Database::open(&db_path)
        .expect("db reopen")
        .messages()
        .list_for_session(&task_id)
        .expect("messages");
    assert!(
        messages
            .iter()
            .any(|message| message.role == "user"
                && message.content.contains("durable background work")),
        "child user prompt must persist across restart: {messages:#?}"
    );
    assert!(
        messages.iter().any(|message| message.role == "assistant"
            && message.content.contains("drained child answer")),
        "child assistant answer must persist across restart: {messages:#?}"
    );

    // A fresh runner with an empty in-memory map (a new process) rejoins purely from the DB.
    let fresh = runner(&catalog, built);
    let rejoined = fresh
        .run_task(TaskRequest {
            agent: None,
            prompt: None,
            description: None,
            task_id: Some(task_id.clone()),
            command: None,
            background: false,
        })
        .await;
    assert!(!rejoined.is_error, "got: {}", rejoined.model_content());
    assert!(
        rejoined.model_content().contains("state: completed"),
        "post-restart rejoin should return completion: {}",
        rejoined.model_content()
    );
    assert!(
        rejoined.model_content().contains("drained child answer"),
        "post-restart rejoin should include the child answer: {}",
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
    let run = runner(
        &catalog,
        config(&dir, db_path, inputs).provider(provider).build(),
    );

    let out = run
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
