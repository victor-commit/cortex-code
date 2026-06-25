//! The `task` tool: delegate a focused prompt to a permitted subagent type.
//!
//! The tool is deliberately a thin surface seam. It validates the target agent against the
//! advertised list, gates `task:<agent>` through the permission engine, then calls the
//! surface-provided [`TaskRunner`](crate::TaskRunner). The CLI runner owns the real durable child
//! session, provider turn, tool registry, and PID cleanup.

use std::collections::BTreeMap;

use futures_util::future::BoxFuture;
use serde_json::{Value, json};

use crate::tool::{TaskRequest, TaskTarget, Tool, ToolContext, ToolOutput};

/// Model-facing task/subagent delegation tool.
pub struct TaskTool {
    targets: Vec<TaskTarget>,
    target_lookup: BTreeMap<String, String>,
}

impl TaskTool {
    /// Build a task tool whose selectable targets have already been permission-filtered by the
    /// surface. The tool still enforces the same target list and `task:<agent>` permission at call
    /// time so a stale or adversarial model call cannot spawn a hidden target.
    #[must_use]
    pub fn new(targets: Vec<TaskTarget>) -> Self {
        let target_lookup = targets
            .iter()
            .map(|target| (normalize_agent_name(&target.name), target.name.clone()))
            .collect();
        Self {
            targets,
            target_lookup,
        }
    }

    fn target_names(&self) -> Vec<String> {
        self.targets
            .iter()
            .map(|target| target.name.clone())
            .collect()
    }
}

impl Tool for TaskTool {
    fn name(&self) -> &str {
        "task"
    }

    fn description(&self) -> &str {
        "Delegate a focused subtask to one of the permitted non-primary subagent types. The \
         subagent runs in its own durable session under that agent's persona and permission rules, \
         and its final result is returned here for you to incorporate into your answer. Use this \
         for read/search/investigation work that benefits from a separate agent. Compatibility \
         parameters: `subagent_type`/`agent`, optional `task_id` resume, `description`, \
         `command`, and `background`."
    }

    fn parameters(&self) -> Value {
        let names = self.target_names().join(", ");
        json!({
            "type": "object",
            "properties": {
                "subagent_type": {
                    "type": "string",
                    "description": format!("The subagent type to run. Available targets in this session: {names}.")
                },
                "subagentType": {
                    "type": "string",
                    "description": "Compatibility alias for `subagent_type`."
                },
                "agent": {
                    "type": "string",
                    "description": "Compatibility alias for `subagent_type`."
                },
                "prompt": {
                    "type": "string",
                    "description": "The complete delegated task prompt for the subagent."
                },
                "description": {
                    "type": "string",
                    "description": "A short (3-5 words) task description used for child-session labeling."
                },
                "task_id": {
                    "type": "string",
                    "description": "Resume or rejoin a previous task session by id."
                },
                "taskId": {
                    "type": "string",
                    "description": "Compatibility alias for `task_id`."
                },
                "command": {
                    "type": "string",
                    "description": "Optional command context that triggered this task."
                },
                "background": {
                    "type": "boolean",
                    "description": "Run asynchronously and return immediately with task state information."
                }
            },
            "anyOf": [
                { "required": ["subagent_type", "prompt"] },
                { "required": ["agent", "prompt"] },
                { "required": ["task_id"] },
                { "required": ["taskId"] }
            ],
            "additionalProperties": false
        })
    }

    fn execute<'a>(&'a self, arguments: Value, ctx: &'a ToolContext) -> BoxFuture<'a, ToolOutput> {
        Box::pin(async move {
            let task_id = string_arg(&arguments, &["task_id", "taskId"]);
            let background = arguments
                .get("background")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let description = string_arg(&arguments, &["description"]);
            let command = string_arg(&arguments, &["command"]);
            let prompt = string_arg(&arguments, &["prompt"]);

            let agent_token = string_arg(&arguments, &["subagent_type", "subagentType", "agent"]);
            let agent = match agent_token {
                Some(token) => {
                    let Some(agent) = self.target_lookup.get(&normalize_agent_name(&token)) else {
                        let available = self.target_names().join(", ");
                        return ToolOutput::error(format!(
                            "task: subagent `{token}` is not available; permitted targets are: {available}"
                        ));
                    };
                    Some(agent.clone())
                }
                None => None,
            };

            if task_id.is_none() && prompt.is_none() {
                return ToolOutput::error(
                    "task: provide either 'prompt' (for a new task) or 'task_id' (to rejoin/resume)",
                );
            }
            if prompt.is_some() && agent.is_none() {
                return ToolOutput::error(
                    "task: the 'subagent_type' argument is required when 'prompt' is provided",
                );
            }

            if let Some(agent) = agent.as_deref() {
                if let Err(message) = ctx.ensure_permitted("task", agent) {
                    return ToolOutput::error(format!("{message} (subagent not spawned)"));
                }
            }

            let Some(runner) = ctx.task_runner() else {
                return ToolOutput::error(
                    "task: subagent execution is not available in this surface",
                );
            };

            runner
                .run_task(TaskRequest {
                    agent,
                    prompt,
                    description,
                    task_id,
                    command,
                    background,
                })
                .await
        })
    }
}

fn string_arg(arguments: &Value, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        arguments
            .get(*key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
    })
}

fn normalize_agent_name(input: &str) -> String {
    input
        .trim()
        .trim_start_matches('@')
        .to_ascii_lowercase()
        .replace([' ', '-'], "_")
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use permissions::{Action, PermissionEngine, Rule, Ruleset};

    use super::*;
    use crate::tool::TaskRunner;

    struct RecordingRunner {
        requests: Mutex<Vec<TaskRequest>>,
    }

    impl RecordingRunner {
        fn new() -> Self {
            Self {
                requests: Mutex::new(Vec::new()),
            }
        }
    }

    impl TaskRunner for RecordingRunner {
        fn run_task<'a>(&'a self, request: TaskRequest) -> BoxFuture<'a, ToolOutput> {
            Box::pin(async move {
                self.requests.lock().expect("lock").push(request.clone());
                ToolOutput::text(format!(
                    "subagent {} answered {}",
                    request.agent.as_deref().unwrap_or("unknown"),
                    request.prompt.as_deref().unwrap_or("(no prompt)")
                ))
            })
        }
    }

    fn tool() -> TaskTool {
        TaskTool::new(vec![
            TaskTarget::new("explore", "Read-only exploration"),
            TaskTarget::new("general", "General purpose"),
        ])
    }

    #[tokio::test]
    async fn task_invokes_the_attached_runner_for_a_permitted_target() {
        let runner = Arc::new(RecordingRunner::new());
        let ctx = ToolContext::new(std::env::temp_dir()).with_task_runner(runner.clone());
        let out = tool()
            .execute(
                json!({"subagent_type":"explore","prompt":"find TODO markers"}),
                &ctx,
            )
            .await;
        assert!(!out.is_error, "got: {}", out.model_content());
        assert!(out.model_content().contains("explore answered"));
        assert_eq!(
            runner.requests.lock().expect("lock").as_slice(),
            &[TaskRequest::new("explore", "find TODO markers")]
        );
    }

    #[tokio::test]
    async fn task_accepts_alias_names_and_resume_fields() {
        let runner = Arc::new(RecordingRunner::new());
        let ctx = ToolContext::new(std::env::temp_dir()).with_task_runner(runner.clone());
        let out = tool()
            .execute(
                json!({
                    "agent":"EXPLORE",
                    "prompt":"resume context",
                    "description":"Resume task",
                    "task_id":"ses_child",
                    "command":"inspect --deep",
                    "background":true
                }),
                &ctx,
            )
            .await;
        assert!(!out.is_error, "got: {}", out.model_content());
        let request = runner.requests.lock().expect("lock")[0].clone();
        assert_eq!(request.agent.as_deref(), Some("explore"));
        assert_eq!(request.task_id.as_deref(), Some("ses_child"));
        assert_eq!(request.command.as_deref(), Some("inspect --deep"));
        assert_eq!(request.description.as_deref(), Some("Resume task"));
        assert!(request.background);
    }

    #[tokio::test]
    async fn task_rejects_targets_that_were_not_advertised() {
        let runner = Arc::new(RecordingRunner::new());
        let ctx = ToolContext::new(std::env::temp_dir()).with_task_runner(runner);
        let out = tool()
            .execute(json!({"subagent_type":"build","prompt":"do work"}), &ctx)
            .await;
        assert!(out.is_error);
        assert!(out.model_content().contains("not available"));
    }

    #[tokio::test]
    async fn task_permission_denial_prevents_spawning() {
        let runner = Arc::new(RecordingRunner::new());
        let engine = PermissionEngine::new(Ruleset::new(vec![Rule::new(
            "task",
            "explore",
            Action::Deny,
        )]));
        let ctx = ToolContext::new(std::env::temp_dir())
            .with_task_runner(runner.clone())
            .with_permissions(Arc::new(engine));
        let out = tool()
            .execute(json!({"subagent_type":"explore","prompt":"do work"}), &ctx)
            .await;
        assert!(out.is_error);
        assert!(out.model_content().contains("permission denied"));
        assert!(
            runner.requests.lock().expect("lock").is_empty(),
            "denied task must not call the runner"
        );
    }

    #[tokio::test]
    async fn task_ask_without_responder_fails_closed_and_prevents_spawning() {
        // VAL-GAP-PERM-004 deterministic registry-level coverage for task ask behavior.
        let runner = Arc::new(RecordingRunner::new());
        let engine = PermissionEngine::new(Ruleset::new(vec![Rule::new(
            "task",
            "explore",
            Action::Ask,
        )]));
        let ctx = ToolContext::new(std::env::temp_dir())
            .with_task_runner(runner.clone())
            .with_permissions(Arc::new(engine));
        let out = tool()
            .execute(json!({"subagent_type":"explore","prompt":"do work"}), &ctx)
            .await;
        assert!(out.is_error);
        assert!(out.model_content().contains("permission denied"));
        assert!(
            runner.requests.lock().expect("lock").is_empty(),
            "ask without responder must fail closed and not spawn a task"
        );
    }

    #[tokio::test]
    async fn task_allows_rejoin_queries_with_task_id_only() {
        let runner = Arc::new(RecordingRunner::new());
        let ctx = ToolContext::new(std::env::temp_dir()).with_task_runner(runner.clone());
        let out = tool().execute(json!({"task_id":"ses_child"}), &ctx).await;
        assert!(!out.is_error, "got: {}", out.model_content());
        let request = runner.requests.lock().expect("lock")[0].clone();
        assert_eq!(request.task_id.as_deref(), Some("ses_child"));
        assert!(request.prompt.is_none());
        assert!(request.agent.is_none());
    }

    #[test]
    fn parameters_advertise_compatibility_fields() {
        let params = tool().parameters();
        assert_eq!(
            params["properties"]["subagent_type"]["type"],
            serde_json::json!("string")
        );
        assert_eq!(
            params["properties"]["background"]["type"],
            serde_json::json!("boolean")
        );
        assert!(
            params["properties"]["subagent_type"]["description"]
                .as_str()
                .unwrap()
                .contains("explore, general")
        );
        assert_eq!(
            params["anyOf"][0]["required"],
            serde_json::json!(["subagent_type", "prompt"])
        );
    }
}
