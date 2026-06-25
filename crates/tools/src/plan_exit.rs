//! The `plan-exit` tool: leave planning mode after approval.

use futures_util::future::BoxFuture;
use serde_json::{Value, json};

use crate::tool::{Tool, ToolContext, ToolOutput};

/// Model-facing advisory tool for leaving planning mode.
pub struct PlanExitTool;

impl Tool for PlanExitTool {
    fn name(&self) -> &str {
        "plan-exit"
    }

    fn description(&self) -> &str {
        "Exit planning mode after the plan is approved. Requires `approved=true`."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "approved": {
                    "type": "boolean",
                    "description": "Set true after the user approves the plan to re-enable mutations."
                }
            },
            "additionalProperties": false
        })
    }

    fn execute<'a>(&'a self, arguments: Value, ctx: &'a ToolContext) -> BoxFuture<'a, ToolOutput> {
        Box::pin(async move {
            if let Err(message) = ctx.ensure_permitted("plan-exit", "planning-mode") {
                return ToolOutput::error(message);
            }

            let approved = arguments
                .get("approved")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            if let Err(message) = ctx.exit_planning_mode(approved) {
                return ToolOutput::error(message);
            }

            ToolOutput::text(
                "plan-exit: planning mode disabled, mutating tools are available again",
            )
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
    async fn deny_rule_blocks_plan_exit_requests() {
        let engine = PermissionEngine::new(Ruleset::new(vec![Rule::new(
            "plan-exit",
            "*",
            Action::Deny,
        )]));
        let ctx = ToolContext::new(std::env::temp_dir()).with_permissions(Arc::new(engine));
        let out = PlanExitTool.execute(json!({}), &ctx).await;
        assert!(out.is_error);
        assert!(out.model_content().contains("permission denied"));
    }
}
