use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use db::Database;
use events::EventBus;
use futures_util::future::BoxFuture;
use llm::{FinishReason, LlmEvent, LlmRequest, LlmStream, Provider, ToolCall, Usage};
use permissions::{Action, PermissionEngine, Rule, Ruleset};
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
            parent_model_id: "google/gemini-2.5-flash".to_owned(),
            formatter_config: format::FormatterConfig::default(),
            lsp_config: lsp::LspConfigSet::defaults(),
            config_inputs: self.inputs,
            project_memory: None,
            bus: Arc::new(EventBus::new()),
        }
    }
}

fn runner(catalog: &SubagentCatalog, config: CliTaskRunnerConfig) -> Arc<crate::CliTaskRunner> {
    catalog.runner(config)
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
