//! The `loop-safeguard` tool: advisory repeated-call guard hook.
//!
//! This tool is invoked internally by the runner when repeated identical tool-call rounds are
//! detected. It provides a real permission-gated hook for the `loop-safeguard` capability.

use futures_util::future::BoxFuture;
use serde_json::{Value, json};

use crate::tool::{Tool, ToolContext, ToolOutput};

/// Internal advisory tool used for repeated-call loop safeguards.
pub struct LoopSafeguardTool;

impl Tool for LoopSafeguardTool {
    fn name(&self) -> &str {
        "loop-safeguard"
    }

    fn description(&self) -> &str {
        "Internal advisory guard for repeated identical tool-call rounds."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "detail": {
                    "type": "string",
                    "description": "Runner-provided loop-safeguard context detail."
                }
            },
            "additionalProperties": false
        })
    }

    fn execute<'a>(&'a self, arguments: Value, ctx: &'a ToolContext) -> BoxFuture<'a, ToolOutput> {
        Box::pin(async move {
            let detail = arguments
                .get("detail")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|detail| !detail.is_empty())
                .unwrap_or("repeated identical tool-call rounds detected");
            if let Err(message) = ctx.ensure_permitted("loop-safeguard", detail) {
                return ToolOutput::error(message);
            }
            ToolOutput::text(format!("loop-safeguard engaged: {detail}"))
        })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use permissions::{Action, PermissionEngine, Rule, Ruleset};

    use super::*;

    #[tokio::test]
    async fn deny_rule_blocks_loop_safeguard_hook() {
        let engine = PermissionEngine::new(Ruleset::new(vec![Rule::new(
            "loop-safeguard",
            "*",
            Action::Deny,
        )]));
        let ctx = ToolContext::new(std::env::temp_dir()).with_permissions(Arc::new(engine));
        let out = LoopSafeguardTool
            .execute(json!({"detail":"repeat"}), &ctx)
            .await;
        assert!(out.is_error);
        assert!(out.model_content().contains("permission denied"));
    }
}
