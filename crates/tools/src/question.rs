//! The `question` tool: request structured user input from interactive surfaces.
//!
//! The full interactive flow (surface prompt + reply wiring) is owned by surface-specific
//! integrations. This base tool provides a real permission-gated product hook so advisory
//! capability policies can deterministically gate `question` calls in every surface.

use futures_util::future::BoxFuture;
use serde_json::{Value, json};

use crate::tool::{Tool, ToolContext, ToolOutput};

/// Model-facing advisory tool for asking user questions.
pub struct QuestionTool;

impl Tool for QuestionTool {
    fn name(&self) -> &str {
        "question"
    }

    fn description(&self) -> &str {
        "Request structured user input for ambiguous requirements or tradeoffs. Use this when \
         progress depends on a direct user answer."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "questions": {
                    "type": "array",
                    "description": "One or more question prompts to ask the user.",
                    "items": {
                        "type": "object",
                        "properties": {
                            "prompt": {
                                "type": "string",
                                "description": "The question text shown to the user."
                            },
                            "options": {
                                "type": "array",
                                "description": "Optional predefined choices.",
                                "items": { "type": "string" }
                            }
                        },
                        "required": ["prompt"],
                        "additionalProperties": false
                    },
                    "minItems": 1
                }
            },
            "required": ["questions"],
            "additionalProperties": false
        })
    }

    fn execute<'a>(&'a self, arguments: Value, ctx: &'a ToolContext) -> BoxFuture<'a, ToolOutput> {
        Box::pin(async move {
            let Some(raw) = arguments.get("questions").and_then(Value::as_array) else {
                return ToolOutput::error("question: the 'questions' argument is required");
            };
            if raw.is_empty() {
                return ToolOutput::error("question: the 'questions' array must not be empty");
            }
            for (index, question) in raw.iter().enumerate() {
                let Some(prompt) = question.get("prompt").and_then(Value::as_str) else {
                    return ToolOutput::error(format!(
                        "question: item {} is missing the required 'prompt' string",
                        index + 1
                    ));
                };
                if prompt.trim().is_empty() {
                    return ToolOutput::error(format!(
                        "question: item {} has an empty 'prompt'",
                        index + 1
                    ));
                }
            }

            if let Err(message) = ctx.ensure_permitted("question", "interactive-user-input") {
                return ToolOutput::error(message);
            }

            ToolOutput::error(
                "question: interactive question flow is not available in this surface",
            )
        })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use permissions::{Action, PermissionEngine, Rule, Ruleset};

    use super::*;

    #[tokio::test]
    async fn deny_rule_blocks_question_requests() {
        let engine =
            PermissionEngine::new(Ruleset::new(vec![Rule::new("question", "*", Action::Deny)]));
        let ctx = ToolContext::new(std::env::temp_dir()).with_permissions(Arc::new(engine));
        let out = QuestionTool
            .execute(json!({"questions":[{"prompt":"Which path?"}]}), &ctx)
            .await;
        assert!(out.is_error);
        assert!(out.model_content().contains("permission denied"));
    }
}
