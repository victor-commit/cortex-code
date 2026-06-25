//! The `plan-exit` tool: advisory planning-mode exit hook.
//!
//! The full planning workflow is owned by surface-specific mode integrations. This tool provides
//! a real permission-gated hook for advisory capability policies.

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
        "Request exit from planning mode after the plan is complete."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {},
            "additionalProperties": false
        })
    }

    fn execute<'a>(&'a self, _arguments: Value, ctx: &'a ToolContext) -> BoxFuture<'a, ToolOutput> {
        Box::pin(async move {
            if let Err(message) = ctx.ensure_permitted("plan-exit", "planning-mode") {
                return ToolOutput::error(message);
            }

            ToolOutput::error(
                "plan-exit: planning mode transitions are not available in this surface",
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
