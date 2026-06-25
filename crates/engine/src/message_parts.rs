//! Build persisted rich message parts from a completed runner turn.
//! Build persisted rich message parts from a completed runner turn.
use std::collections::HashMap;

use llm::Usage;
use protocol::{MessagePart, MessagePartBody, StepPartPhase, ToolPartState, ToolResultPart};

use crate::{
    RunOutcome, RunnerEvent, agent_part_id, compaction_part_id, file_part_id, reasoning_part_id,
    retry_part_id, step_finish_part_id, step_part_id, text_part_id, tool_pending_part_id,
    tool_running_part_id, tool_terminal_part_id, usage_part_id,
};

/// Build rich assistant message parts from streamed runner events and the final turn outcome.
#[must_use]
pub fn build_assistant_message_parts(
    model_id: &str,
    agent_name: &str,
    turn_id: &str,
    events: &[RunnerEvent],
    outcome: &RunOutcome,
    part_timestamps: &HashMap<String, String>,
) -> Vec<MessagePart> {
    let mut parts = Vec::new();
    let agent_id = agent_part_id(turn_id);
    parts.push(MessagePart {
        id: agent_id.clone(),
        created_at: part_timestamps.get(&agent_id).cloned(),
        body: MessagePartBody::Agent {
            name: agent_name.to_owned(),
            mode: None,
            model: Some(model_id.to_owned()),
        },
    });
    let step_start_id = step_part_id(turn_id);
    parts.push(MessagePart {
        id: step_start_id.clone(),
        created_at: part_timestamps.get(&step_start_id).cloned(),
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
    let mut usage_count = 0usize;
    let mut retry_count = 0usize;

    for event in events {
        match event {
            RunnerEvent::TextDelta { text: delta } => text.push_str(delta),
            RunnerEvent::Reasoning { text: delta } => reasoning.push_str(delta),
            RunnerEvent::ToolCall {
                id,
                name,
                arguments,
            } => {
                let pending_id = tool_pending_part_id(turn_id, id);
                parts.push(MessagePart {
                    id: pending_id.clone(),
                    created_at: part_timestamps.get(&pending_id).cloned(),
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
                let running_id = tool_running_part_id(turn_id, id);
                parts.push(MessagePart {
                    id: running_id.clone(),
                    created_at: part_timestamps.get(&running_id).cloned(),
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
                let terminal_id = tool_terminal_part_id(turn_id, id, *is_error);
                parts.push(MessagePart {
                    id: terminal_id.clone(),
                    created_at: part_timestamps.get(&terminal_id).cloned(),
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
                        .enumerate()
                        .filter_map(|(ordinal, part)| {
                            file_part_from_tool_attachment(
                                turn_id,
                                id,
                                ordinal + 1,
                                part,
                                part_timestamps,
                            )
                        }),
                );
            }
            RunnerEvent::Retry {
                attempt,
                max_attempts,
                delay_ms,
                error_kind,
                classification,
                message,
            } => {
                retry_count += 1;
                let retry_id = retry_part_id(turn_id, retry_count);
                parts.push(MessagePart {
                    id: retry_id.clone(),
                    created_at: part_timestamps.get(&retry_id).cloned(),
                    body: MessagePartBody::Retry {
                        attempt: *attempt,
                        max_attempts: *max_attempts,
                        delay_ms: *delay_ms,
                        error_kind: error_kind.clone(),
                        classification: classification.clone(),
                        message: message.clone(),
                    },
                });
            }
            RunnerEvent::Usage(usage) => {
                usage_count += 1;
                parts.push(usage_part(turn_id, usage, usage_count, part_timestamps));
            }
            RunnerEvent::Error { .. }
            | RunnerEvent::Done { .. }
            | RunnerEvent::ToolSetChanged { .. } => {}
        }
    }

    if !reasoning.trim().is_empty() {
        let reasoning_id = reasoning_part_id(turn_id);
        parts.push(MessagePart {
            id: reasoning_id.clone(),
            created_at: part_timestamps.get(&reasoning_id).cloned(),
            body: MessagePartBody::Reasoning {
                text: reasoning.trim().to_owned(),
            },
        });
    }
    if !text.trim().is_empty() {
        let text_id = text_part_id(turn_id);
        parts.push(MessagePart {
            id: text_id.clone(),
            created_at: part_timestamps.get(&text_id).cloned(),
            body: MessagePartBody::Text {
                text: text.trim().to_owned(),
            },
        });
    }
    for (index, compaction) in outcome.compactions.iter().enumerate() {
        let compaction_id = compaction_part_id(turn_id, index + 1);
        parts.push(MessagePart {
            id: compaction_id.clone(),
            created_at: part_timestamps.get(&compaction_id).cloned(),
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
    let step_finish_id = step_finish_part_id(turn_id, outcome.steps);
    parts.push(MessagePart {
        id: step_finish_id.clone(),
        created_at: part_timestamps.get(&step_finish_id).cloned(),
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

fn usage_part(
    turn_id: &str,
    usage: &Usage,
    ordinal: usize,
    part_timestamps: &HashMap<String, String>,
) -> MessagePart {
    let usage_id = usage_part_id(turn_id, ordinal);
    MessagePart {
        id: usage_id.clone(),
        created_at: part_timestamps.get(&usage_id).cloned(),
        body: MessagePartBody::Usage {
            input_tokens: usage.input_tokens,
            output_tokens: usage.output_tokens,
            total_tokens: usage.total_tokens,
            cost: usage.cost,
        },
    }
}

fn file_part_from_tool_attachment(
    turn_id: &str,
    call_id: &str,
    ordinal: usize,
    part: &ToolResultPart,
    part_timestamps: &HashMap<String, String>,
) -> Option<MessagePart> {
    match part {
        ToolResultPart::Image { media_type, path }
        | ToolResultPart::Document { media_type, path } => {
            let file_id = file_part_id(turn_id, call_id, ordinal);
            Some(MessagePart {
                id: file_id.clone(),
                created_at: part_timestamps.get(&file_id).cloned(),
                body: MessagePartBody::File {
                    path: path.clone(),
                    media_type: Some(media_type.clone()),
                    source: Some("tool_result".to_owned()),
                },
            })
        }
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
        let parts = build_assistant_message_parts(
            "google/gemini-2.5-flash",
            "build",
            "msg_turn_1",
            &events,
            &outcome,
            &HashMap::new(),
        );
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
        assert!(
            parts.iter().all(|part| !part.id.is_empty()),
            "every persisted part has a stable id"
        );
    }
}
