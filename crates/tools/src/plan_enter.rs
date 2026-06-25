//! The `plan-enter` tool: advisory planning-mode entry hook.
//!
//! The full planning workflow is owned by surface-specific mode integrations. This tool provides
//! a real permission-gated hook for advisory capability policies.

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
        "Request entry into planning mode before implementation."
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
            if let Err(message) = ctx.ensure_permitted("plan-enter", "planning-mode") {
                return ToolOutput::error(message);
            }

            ToolOutput::error(
                "plan-enter: planning mode transitions are not available in this surface",
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
