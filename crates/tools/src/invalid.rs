//! The `invalid` tool: model-readable fallback for malformed or unknown tool calls.

use futures_util::future::BoxFuture;
use serde_json::{Value, json};

use crate::tool::{Tool, ToolContext, ToolOutput};

/// Model-facing invalid-tool fallback.
pub struct InvalidTool;

impl Tool for InvalidTool {
    fn name(&self) -> &str {
        "invalid"
    }

    fn description(&self) -> &str {
        "Fallback tool result for unknown or malformed tool calls. Use this to report the issue \
         and recover without crashing the turn."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "tool": {
                    "type": "string",
                    "description": "The invalid tool name that was requested."
                },
                "reason": {
                    "type": "string",
                    "description": "Why the tool call was invalid."
                },
                "arguments": {
                    "type": "string",
                    "description": "Original argument payload, if available."
                }
            },
            "required": ["tool", "reason"],
            "additionalProperties": false
        })
    }

    fn execute<'a>(&'a self, arguments: Value, _ctx: &'a ToolContext) -> BoxFuture<'a, ToolOutput> {
        Box::pin(async move {
            let tool = arguments
                .get("tool")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            let reason = arguments
                .get("reason")
                .and_then(Value::as_str)
                .unwrap_or("invalid tool call");
            let mut message = format!("invalid tool call `{tool}`: {reason}");
            if let Some(arguments) = arguments.get("arguments").and_then(Value::as_str)
                && !arguments.trim().is_empty()
            {
                message.push_str("\narguments: ");
                message.push_str(arguments);
            }
            ToolOutput::error(message)
        })
    }
}

/// Build a standardized invalid-tool output for a call to an unknown tool.
#[must_use]
pub fn unknown_tool_output(name: &str) -> ToolOutput {
    ToolOutput::error(format!(
        "invalid tool call `{name}`: unknown tool name, choose one from the advertised tool list"
    ))
}

/// Build a standardized invalid-tool output for malformed argument JSON.
#[must_use]
pub fn invalid_arguments_output(
    name: &str,
    arguments: &str,
    err: &serde_json::Error,
) -> ToolOutput {
    ToolOutput::error(format!(
        "invalid tool call `{name}`: invalid arguments JSON ({err})\narguments: {arguments}"
    ))
}
