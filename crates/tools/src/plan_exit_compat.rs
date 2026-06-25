//! The `plan_exit` tool: compatibility alias for `plan-exit`.

use futures_util::future::BoxFuture;
use serde_json::{Value, json};

use crate::tool::{Tool, ToolContext, ToolOutput};

/// Model-facing compatibility alias for leaving planning mode.
pub struct PlanExitCompatTool;

impl Tool for PlanExitCompatTool {
    fn name(&self) -> &str {
        "plan_exit"
    }

    fn description(&self) -> &str {
        "Compatibility alias for plan-exit. Exit planning mode after approval (approved=true)."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "approved": {
                    "type": "boolean",
                    "description": "Set true after the user approves the plan."
                }
            },
            "additionalProperties": false
        })
    }

    fn execute<'a>(&'a self, arguments: Value, ctx: &'a ToolContext) -> BoxFuture<'a, ToolOutput> {
        Box::pin(async move {
            if let Err(message) = ctx.ensure_permitted("plan_exit", "planning-mode") {
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
                "plan_exit: planning mode disabled, mutating tools are available again",
            )
        })
    }
}
