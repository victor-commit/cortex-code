//! Build persisted rich message parts from a completed runner turn.

use llm::Usage;
use protocol::{MessagePart, MessagePartBody, StepPartPhase, ToolPartState, ToolResultPart};

use crate::{RunOutcome, RunnerEvent};

/// Build rich assistant message parts from streamed runner events and the final turn outcome.
#[must_use]
pub fn build_assistant_message_parts(
    model_id: &str,
    agent_name: &str,
    events: &[RunnerEvent],
    outcome: &RunOutcome,
) -> Vec<MessagePart> {
    let mut parts = Vec::new();
    parts.push(MessagePart {
        id: String::new(),
        created_at: None,
        body: MessagePartBody::Agent {
            name: agent_name.to_owned(),
            mode: None,
            model: Some(model_id.to_owned()),
        },
    });
    parts.push(MessagePart {
        id: String::new(),
        created_at: None,
        body: MessagePartBody::Step {
            phase: StepPartPhase::Start,
            step: 1,
            model: Some(model_id.to_owned()),
            finish_reason: None,
            input_tokens: None,
            output_tokens: None,
            total_tokens: None,
            cost: None,
        },
    });

    let mut text = String::new();
    let mut reasoning = String::new();

    for event in events {
        match event {
            RunnerEvent::TextDelta { text: delta } => text.push_str(delta),
            RunnerEvent::Reasoning { text: delta } => reasoning.push_str(delta),
            RunnerEvent::ToolCall {
                id,
                name,
                arguments,
            } => {
                parts.push(MessagePart {
                    id: String::new(),
                    created_at: None,
                    body: MessagePartBody::Tool {
                        call_id: id.clone(),
                        name: name.clone(),
                        state: ToolPartState::Pending,
                        input: Some(arguments.clone()),
                        output: None,
                        attachments: Vec::new(),
                        metadata: serde_json::Value::Null,
                    },
                });
                parts.push(MessagePart {
                    id: String::new(),
                    created_at: None,
                    body: MessagePartBody::Tool {
                        call_id: id.clone(),
                        name: name.clone(),
                        state: ToolPartState::Running,
                        input: Some(arguments.clone()),
                        output: None,
                        attachments: Vec::new(),
                        metadata: serde_json::Value::Null,
                    },
                });
            }
            RunnerEvent::ToolResult {
                id,
                name,
                parts: result_parts,
                is_error,
            } => {
                let output = result_parts
                    .iter()
                    .filter_map(ToolResultPart::as_text)
                    .collect::<Vec<_>>()
                    .join("\n");
                parts.push(MessagePart {
                    id: String::new(),
                    created_at: None,
                    body: MessagePartBody::Tool {
                        call_id: id.clone(),
                        name: name.clone(),
                        state: if *is_error {
                            ToolPartState::Error
                        } else {
                            ToolPartState::Completed
                        },
                        input: None,
                        output: (!output.is_empty()).then_some(output),
                        attachments: result_parts.clone(),
                        metadata: serde_json::json!({ "is_error": is_error }),
                    },
                });
                parts.extend(
                    result_parts
                        .iter()
                        .filter_map(file_part_from_tool_attachment),
                );
            }
            RunnerEvent::Retry {
                attempt,
                max_attempts,
                delay_ms,
                error_kind,
                classification,
                message,
            } => parts.push(MessagePart {
                id: String::new(),
                created_at: None,
                body: MessagePartBody::Retry {
                    attempt: *attempt,
                    max_attempts: *max_attempts,
                    delay_ms: *delay_ms,
                    error_kind: error_kind.clone(),
                    classification: classification.clone(),
                    message: message.clone(),
                },
            }),
            RunnerEvent::Usage(usage) => parts.push(usage_part(usage)),
            RunnerEvent::Error { .. }
            | RunnerEvent::Done { .. }
            | RunnerEvent::ToolSetChanged { .. } => {}
        }
    }

    if !reasoning.trim().is_empty() {
        parts.push(MessagePart::reasoning(reasoning.trim().to_owned()));
    }
    if !text.trim().is_empty() {
        parts.push(MessagePart::text(text.trim().to_owned()));
    }
    for compaction in &outcome.compactions {
        parts.push(MessagePart {
            id: String::new(),
            created_at: None,
            body: MessagePartBody::Compaction {
                summary: compaction.summary.clone(),
                tail_start_id: compaction.tail_start_id.clone(),
                continued_request: compaction.continued_request,
            },
        });
    }
    let (input_tokens, output_tokens, total_tokens, cost) =
        outcome
            .usage
            .as_ref()
            .map_or((None, None, None, None), |usage| {
                (
                    Some(usage.input_tokens),
                    Some(usage.output_tokens),
                    Some(usage.total_tokens),
                    usage.cost,
                )
            });
    parts.push(MessagePart {
        id: String::new(),
        created_at: None,
        body: MessagePartBody::Step {
            phase: StepPartPhase::Finish,
            step: outcome.steps,
            model: Some(model_id.to_owned()),
            finish_reason: Some(finish_reason_name(&outcome.finish_reason)),
            input_tokens,
            output_tokens,
            total_tokens,
            cost,
        },
    });

    parts
}

fn usage_part(usage: &Usage) -> MessagePart {
    MessagePart {
        id: String::new(),
        created_at: None,
        body: MessagePartBody::Usage {
            input_tokens: usage.input_tokens,
            output_tokens: usage.output_tokens,
            total_tokens: usage.total_tokens,
            cost: usage.cost,
        },
    }
}

fn file_part_from_tool_attachment(part: &ToolResultPart) -> Option<MessagePart> {
    match part {
        ToolResultPart::Image { media_type, path }
        | ToolResultPart::Document { media_type, path } => Some(MessagePart {
            id: String::new(),
            created_at: None,
            body: MessagePartBody::File {
                path: path.clone(),
                media_type: Some(media_type.clone()),
                source: Some("tool_result".to_owned()),
            },
        }),
        ToolResultPart::Text { .. } => None,
    }
}

fn finish_reason_name(reason: &llm::FinishReason) -> String {
    serde_json::to_value(reason)
        .ok()
        .and_then(|value| value.as_str().map(ToOwned::to_owned))
        .unwrap_or_else(|| format!("{reason:?}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use llm::FinishReason;

    #[test]
    fn builds_tool_lifecycle_and_usage_parts() {
        let events = vec![
            RunnerEvent::Reasoning {
                text: "thinking".to_owned(),
            },
            RunnerEvent::ToolCall {
                id: "call_1".to_owned(),
                name: "todo".to_owned(),
                arguments: "{\"todos\":[]}".to_owned(),
            },
            RunnerEvent::ToolResult {
                id: "call_1".to_owned(),
                name: "todo".to_owned(),
                parts: vec![ToolResultPart::text("ok")],
                is_error: false,
            },
            RunnerEvent::Usage(Usage {
                input_tokens: 3,
                output_tokens: 2,
                total_tokens: 5,
                cost: Some(0.000001),
            }),
            RunnerEvent::Done {
                reason: FinishReason::Stop,
                steps: 2,
            },
        ];
        let outcome = RunOutcome {
            message: llm::Message::assistant("done"),
            usage: Some(Usage {
                input_tokens: 3,
                output_tokens: 2,
                total_tokens: 5,
                cost: Some(0.000001),
            }),
            finish_reason: FinishReason::Stop,
            steps: 2,
            compactions: Vec::new(),
            error: None,
        };
        let parts =
            build_assistant_message_parts("google/gemini-2.5-flash", "build", &events, &outcome);
        assert!(parts.iter().any(|part| matches!(
            part.body,
            MessagePartBody::Tool {
                state: ToolPartState::Pending,
                ..
            }
        )));
        assert!(parts.iter().any(|part| matches!(
            part.body,
            MessagePartBody::Tool {
                state: ToolPartState::Running,
                ..
            }
        )));
        assert!(parts.iter().any(|part| matches!(
            part.body,
            MessagePartBody::Tool {
                state: ToolPartState::Completed,
                ..
            }
        )));
        assert!(
            parts
                .iter()
                .any(|part| matches!(part.body, MessagePartBody::Usage { .. }))
        );
        assert!(parts.iter().any(|part| matches!(
            part.body,
            MessagePartBody::Step {
                phase: StepPartPhase::Finish,
                ..
            }
        )));
    }
}
