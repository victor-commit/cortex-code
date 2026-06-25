//! The `plan-enter` tool: compatibility alias for enabling planning mode.

use futures_util::future::BoxFuture;
use serde_json::{Value, json};

use crate::tool::{Tool, ToolContext, ToolOutput};

/// Model-facing advisory tool for entering planning mode.
pub struct PlanEnterTool;

impl Tool for PlanEnterTool {
    fn name(&self) -> &str {
        "plan-enter"
    }

    fn description(&self) -> &str {
        "Enter planning mode (compatibility alias for `plan`). While planning mode is active, \
         mutating tools are blocked until `plan-exit` or `plan_exit` succeeds."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "plan": {
                    "type": "string",
                    "description": "Optional short plan summary for transcript clarity."
                }
            },
            "additionalProperties": false
        })
    }

    fn execute<'a>(&'a self, arguments: Value, ctx: &'a ToolContext) -> BoxFuture<'a, ToolOutput> {
        Box::pin(async move {
            if let Err(message) = ctx.ensure_permitted("plan-enter", "planning-mode") {
                return ToolOutput::error(message);
            }

            ctx.enter_planning_mode();
            let summary = arguments
                .get("plan")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|summary| !summary.is_empty());
            let message = match summary {
                Some(summary) => format!(
                    "plan-enter: planning mode enabled. Plan summary:\n{summary}\nUse plan-exit or plan_exit with approved=true after approval."
                ),
                None => "plan-enter: planning mode enabled. Use plan-exit or plan_exit with approved=true after approval.".to_owned(),
            };
            ToolOutput::text(message)
        })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use permissions::{Action, PermissionEngine, Rule, Ruleset};
    use serde_json::json;

    use super::*;

    #[tokio::test]
    async fn deny_rule_blocks_plan_enter_requests() {
        let engine = PermissionEngine::new(Ruleset::new(vec![Rule::new(
            "plan-enter",
            "*",
            Action::Deny,
        )]));
        let ctx = ToolContext::new(std::env::temp_dir()).with_permissions(Arc::new(engine));
        let out = PlanEnterTool.execute(json!({}), &ctx).await;
        assert!(out.is_error);
        assert!(out.model_content().contains("permission denied"));
    }
}
