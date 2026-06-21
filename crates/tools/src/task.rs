//! The `task` tool: delegate a focused prompt to a permitted subagent type.
//!
//! The tool is deliberately a thin surface seam. It validates the target agent against the
//! advertised list, gates `task:<agent>` through the permission engine, then calls the
//! surface-provided [`TaskRunner`](crate::TaskRunner). The CLI runner owns the real durable child
//! session, provider turn, tool registry, and PID cleanup.

use std::collections::BTreeSet;

use futures_util::future::BoxFuture;
use serde_json::{Value, json};

use crate::tool::{TaskRequest, TaskTarget, Tool, ToolContext, ToolOutput};

/// Model-facing task/subagent delegation tool.
pub struct TaskTool {
    targets: Vec<TaskTarget>,
    target_names: BTreeSet<String>,
}

impl TaskTool {
    /// Build a task tool whose selectable targets have already been permission-filtered by the
    /// surface. The tool still enforces the same target list and `task:<agent>` permission at call
    /// time so a stale or adversarial model call cannot spawn a hidden target.
    #[must_use]
    pub fn new(targets: Vec<TaskTarget>) -> Self {
        let target_names = targets.iter().map(|target| target.name.clone()).collect();
        Self {
            targets,
            target_names,
        }
    }

    fn target_names(&self) -> Vec<String> {
        self.targets
            .iter()
            .map(|target| target.name.clone())
            .collect()
    }

    fn target_listing(&self) -> String {
        self.targets
            .iter()
            .map(|target| format!("{}: {}", target.name, target.description))
            .collect::<Vec<_>>()
            .join("; ")
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
         for read/search/investigation work that benefits from a separate agent."
    }

    fn parameters(&self) -> Value {
        let names = self.target_names();
        json!({
            "type": "object",
            "properties": {
                "agent": {
                    "type": "string",
                    "enum": names,
                    "description": format!("The permitted subagent type to run. Available targets: {}.", self.target_listing())
                },
                "prompt": {
                    "type": "string",
                    "description": "The complete delegated task prompt for the subagent."
                }
            },
            "required": ["agent", "prompt"],
            "additionalProperties": false
        })
    }

    fn execute<'a>(&'a self, arguments: Value, ctx: &'a ToolContext) -> BoxFuture<'a, ToolOutput> {
        Box::pin(async move {
            let Some(agent) = arguments.get("agent").and_then(Value::as_str) else {
                return ToolOutput::error("task: the 'agent' argument is required");
            };
            let agent = agent.trim();
            if agent.is_empty() {
                return ToolOutput::error("task: the 'agent' argument must not be empty");
            }
            if !self.target_names.contains(agent) {
                let available = self.target_names().join(", ");
                return ToolOutput::error(format!(
                    "task: subagent `{agent}` is not available; permitted targets are: {available}"
                ));
            }

            let Some(prompt) = arguments.get("prompt").and_then(Value::as_str) else {
                return ToolOutput::error("task: the 'prompt' argument is required");
            };
            let prompt = prompt.trim();
            if prompt.is_empty() {
                return ToolOutput::error("task: the 'prompt' argument must not be empty");
            }

            if !ctx.permits("task", agent) {
                return ToolOutput::error(format!(
                    "task: permission denied for subagent `{agent}` (subagent not spawned)"
                ));
            }

            let Some(runner) = ctx.task_runner() else {
                return ToolOutput::error(
                    "task: subagent execution is not available in this surface",
                );
            };
            runner
                .run_task(TaskRequest::new(agent.to_owned(), prompt.to_owned()))
                .await
        })
    }
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
                    request.agent, request.prompt
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
                json!({"agent":"explore","prompt":"find TODO markers"}),
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
    async fn task_rejects_targets_that_were_not_advertised() {
        let runner = Arc::new(RecordingRunner::new());
        let ctx = ToolContext::new(std::env::temp_dir()).with_task_runner(runner);
        let out = tool()
            .execute(json!({"agent":"build","prompt":"do work"}), &ctx)
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
            .execute(json!({"agent":"explore","prompt":"do work"}), &ctx)
            .await;
        assert!(out.is_error);
        assert!(out.model_content().contains("permission denied"));
        assert!(
            runner.requests.lock().expect("lock").is_empty(),
            "denied task must not call the runner"
        );
    }

    #[test]
    fn parameters_advertise_exactly_the_permitted_target_enum() {
        let params = tool().parameters();
        let targets: Vec<&str> = params["properties"]["agent"]["enum"]
            .as_array()
            .expect("enum array")
            .iter()
            .filter_map(Value::as_str)
            .collect();
        assert_eq!(targets, vec!["explore", "general"]);
        assert!(
            params["properties"]["agent"]["description"]
                .as_str()
                .unwrap()
                .contains("explore: Read-only exploration")
        );
    }
}
