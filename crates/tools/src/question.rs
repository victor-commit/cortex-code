//! The `question` tool: request structured user input from interactive surfaces.
//!
//! Interactive surfaces (TUI/server) route this through their existing permission-question modal
//! flow so a user can provide an answer and the turn resumes with that answer. Headless runs are
//! non-interactive and fail closed with a clear cannot-ask error.

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

            if !ctx.interactive_questions_enabled() {
                return ToolOutput::error(
                    "question: cannot ask for user input in this non-interactive surface",
                );
            }

            let subject = render_question_subject(raw);
            let decision = ctx.permission_check("question", &subject);
            if !decision.allowed {
                return ToolOutput::error(decision.message.unwrap_or_else(|| {
                    "question: request was denied before the question could be answered".to_owned()
                }));
            }

            let answer = decision.feedback.unwrap_or_default();
            if answer.trim().is_empty() {
                return ToolOutput::error(
                    "question: no answer was provided, include reply text when approving",
                );
            }

            ToolOutput::text(format!("question: user answer\n{answer}"))
        })
    }
}

fn render_question_subject(raw: &[Value]) -> String {
    let mut rendered = String::new();
    for (index, question) in raw.iter().enumerate() {
        let prompt = question
            .get("prompt")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim();
        if prompt.is_empty() {
            continue;
        }
        if !rendered.is_empty() {
            rendered.push('\n');
        }
        rendered.push_str(&format!("Q{}: {prompt}", index + 1));
        if let Some(options) = question.get("options").and_then(Value::as_array) {
            let opts = options
                .iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|option| !option.is_empty())
                .collect::<Vec<_>>();
            if !opts.is_empty() {
                rendered.push_str("\nOptions: ");
                rendered.push_str(&opts.join(" | "));
            }
        }
    }
    if rendered.is_empty() {
        "interactive question".to_owned()
    } else {
        rendered
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use permissions::{
        Action, Decision, PermissionEngine, PermissionRequest, Responder, Rule, Ruleset,
        ScopedDecision,
    };

    use super::*;

    struct AnswerResponder;

    impl Responder for AnswerResponder {
        fn respond(&self, _request: &PermissionRequest) -> Decision {
            Decision::Allow
        }

        fn respond_scoped(&self, _request: &PermissionRequest) -> ScopedDecision {
            ScopedDecision::once(Decision::Allow).with_feedback("Use option B.")
        }
    }

    struct DenyResponder;

    impl Responder for DenyResponder {
        fn respond(&self, _request: &PermissionRequest) -> Decision {
            Decision::Deny
        }
    }

    #[tokio::test]
    async fn headless_surface_fails_closed() {
        let ctx = ToolContext::new(std::env::temp_dir());
        let out = QuestionTool
            .execute(json!({"questions":[{"prompt":"Which path?"}]}), &ctx)
            .await;
        assert!(out.is_error);
        assert!(out.model_content().contains("non-interactive"));
    }

    #[tokio::test]
    async fn interactive_question_returns_user_feedback_answer() {
        let engine =
            PermissionEngine::new(Ruleset::new(vec![Rule::new("question", "*", Action::Ask)]))
                .with_responder(Arc::new(AnswerResponder));
        let ctx = ToolContext::new(std::env::temp_dir())
            .with_permissions(Arc::new(engine))
            .with_interactive_questions(true);
        let out = QuestionTool
            .execute(
                json!({"questions":[{"prompt":"Which path?","options":["A","B"]}]}),
                &ctx,
            )
            .await;
        assert!(!out.is_error, "{}", out.model_content());
        assert!(out.model_content().contains("Use option B."));
    }

    #[tokio::test]
    async fn deny_rule_blocks_interactive_question_requests() {
        let engine =
            PermissionEngine::new(Ruleset::new(vec![Rule::new("question", "*", Action::Ask)]))
                .with_responder(Arc::new(DenyResponder));
        let ctx = ToolContext::new(std::env::temp_dir())
            .with_permissions(Arc::new(engine))
            .with_interactive_questions(true);
        let out = QuestionTool
            .execute(json!({"questions":[{"prompt":"Which path?"}]}), &ctx)
            .await;
        assert!(out.is_error);
        assert!(out.model_content().contains("permission denied"));
    }
}
