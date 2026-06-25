//! The `plan` tool: enter planning mode.

use futures_util::future::BoxFuture;
use serde_json::{Value, json};

use crate::tool::{Tool, ToolContext, ToolOutput};

/// Model-facing planning mode tool.
pub struct PlanTool;

impl Tool for PlanTool {
    fn name(&self) -> &str {
        "plan"
    }

    fn description(&self) -> &str {
        "Enter planning mode. While planning mode is active, mutating tools are blocked until \
         plan-exit/plan_exit succeeds."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "plan": {
                    "type": "string",
                    "description": "Optional short plan summary."
                }
            },
            "additionalProperties": false
        })
    }

    fn execute<'a>(&'a self, arguments: Value, ctx: &'a ToolContext) -> BoxFuture<'a, ToolOutput> {
        Box::pin(async move {
            if let Err(message) = ctx.ensure_permitted("plan", "planning-mode") {
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
                    "plan: planning mode enabled. Plan summary:\n{summary}\nUse plan-exit or plan_exit with approved=true after approval."
                ),
                None => "plan: planning mode enabled. Use plan-exit or plan_exit with approved=true after approval.".to_owned(),
            };
            ToolOutput::text(message)
        })
    }
}
