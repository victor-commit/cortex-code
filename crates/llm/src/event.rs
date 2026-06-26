//! The single canonical normalized event union every provider adapter emits.
//!
//! Every native protocol (OpenAI Chat, Responses, Anthropic, Gemini, OpenAI-compatible) is
//! normalized into an ordered stream of [`LlmEvent`] values so the runner consumes one vocabulary
//! regardless of wire format. A well-formed turn streams its assistant text as [`LlmEvent::TextDelta`]
//! fragments (their concatenation equals the full assistant message), assembles any tool-call
//! argument fragments into a single complete [`ToolCall`], and always terminates with exactly one
//! [`LlmEvent::Done`].

use serde::{Deserialize, Serialize};

/// One normalized streaming event from a provider turn.
///
/// The variant set is the canonical union shared across every adapter. Ordering within a turn is
/// significant: text/reasoning deltas arrive in generation order, assembled tool calls and the
/// usage summary arrive after the model's content, and a single [`LlmEvent::Done`] marks the end.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LlmEvent {
    /// A fragment of assistant-visible text. Concatenating every `TextDelta` in arrival order
    /// reconstructs the complete assistant message.
    TextDelta {
        /// The text fragment.
        text: String,
    },
    /// A fragment of model reasoning/thinking content (kept distinct from user-visible text).
    Reasoning {
        /// The reasoning fragment.
        text: String,
    },
    /// A complete tool call whose streamed argument fragments have been assembled into a single
    /// JSON `arguments` string.
    ToolCall(ToolCall),
    /// The result of executing a tool call, re-injected into the turn. Provider adapters never
    /// originate this; it exists so the union is complete across the system.
    ToolResult(ToolResult),
    /// Token usage (and optional cost) for the turn.
    Usage(Usage),
    /// A normalized provider/transport error that surfaced mid-stream. A terminal
    /// [`LlmEvent::Done`] still follows so consumers always observe an end marker.
    Error {
        /// Human-readable, secret-scrubbed detail.
        message: String,
        /// The stable error class (see [`crate::LlmError::kind`]).
        kind: String,
    },
    /// The single terminal event of a turn, carrying why generation stopped.
    Done {
        /// Why the turn ended.
        reason: FinishReason,
    },
}

/// A complete tool call requested by the model with its arguments fully assembled.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolCall {
    /// Provider-assigned call identifier, echoed back with the tool result.
    pub id: String,
    /// The tool/function name to invoke.
    pub name: String,
    /// The complete arguments as a JSON-encoded string (assembled from streamed fragments).
    pub arguments: String,
}

/// The result of executing a tool call, addressed back to the originating [`ToolCall::id`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolResult {
    /// The [`ToolCall::id`] this result answers.
    pub id: String,
    /// The tool output content.
    pub content: String,
}

/// Token usage (and optional cost) reported for a turn.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Usage {
    /// Prompt/input tokens consumed.
    pub input_tokens: u32,
    /// Completion/output tokens produced.
    pub output_tokens: u32,
    /// Total tokens (input + output) as reported by the provider.
    pub total_tokens: u32,
    /// Prompt-cache write tokens (cache creation), when the provider reports them.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_creation_input_tokens: Option<u32>,
    /// Prompt-cache read tokens, when the provider reports them.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_read_input_tokens: Option<u32>,
    /// Monetary cost of the turn when the provider reports it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost: Option<f64>,
}

/// Why a turn stopped generating.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FinishReason {
    /// The model emitted a natural stop.
    Stop,
    /// Generation hit the maximum token length.
    Length,
    /// The model requested one or more tool calls.
    ToolCalls,
    /// Output was halted by a content filter.
    ContentFilter,
    /// The turn ended because of a normalized error event.
    Error,
    /// Any other provider-specific stop reason, preserved verbatim.
    Other(String),
}

impl FinishReason {
    /// Map an OpenAI-style `finish_reason` string into a normalized [`FinishReason`].
    ///
    /// `saw_tool_calls` disambiguates the absent-reason case: a turn that streamed tool-call
    /// fragments but reported no terminal reason is treated as [`FinishReason::ToolCalls`].
    #[must_use]
    pub fn from_chat(reason: Option<&str>, saw_tool_calls: bool) -> Self {
        match reason {
            Some("stop") => FinishReason::Stop,
            Some("length") => FinishReason::Length,
            Some("tool_calls") => FinishReason::ToolCalls,
            Some("content_filter") => FinishReason::ContentFilter,
            Some(other) => FinishReason::Other(other.to_owned()),
            None if saw_tool_calls => FinishReason::ToolCalls,
            None => FinishReason::Stop,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_delta_serializes_with_a_type_discriminator() {
        let value = serde_json::to_value(LlmEvent::TextDelta {
            text: "hi".to_owned(),
        })
        .expect("serialize");
        assert_eq!(value["type"], "text_delta");
        assert_eq!(value["text"], "hi");
    }

    #[test]
    fn tool_call_flattens_its_fields_under_the_discriminator() {
        let value = serde_json::to_value(LlmEvent::ToolCall(ToolCall {
            id: "call_1".to_owned(),
            name: "get_weather".to_owned(),
            arguments: r#"{"city":"SF"}"#.to_owned(),
        }))
        .expect("serialize");
        assert_eq!(value["type"], "tool_call");
        assert_eq!(value["id"], "call_1");
        assert_eq!(value["name"], "get_weather");
        assert_eq!(value["arguments"], r#"{"city":"SF"}"#);
    }

    #[test]
    fn events_round_trip_through_json() {
        let events = vec![
            LlmEvent::TextDelta {
                text: "a".to_owned(),
            },
            LlmEvent::Reasoning {
                text: "think".to_owned(),
            },
            LlmEvent::ToolResult(ToolResult {
                id: "call_1".to_owned(),
                content: "ok".to_owned(),
            }),
            LlmEvent::Usage(Usage {
                input_tokens: 3,
                output_tokens: 4,
                total_tokens: 7,
                cache_creation_input_tokens: Some(2),
                cache_read_input_tokens: Some(1),
                cost: Some(0.0001),
            }),
            LlmEvent::Error {
                message: "boom".to_owned(),
                kind: "upstream".to_owned(),
            },
            LlmEvent::Done {
                reason: FinishReason::Stop,
            },
        ];
        let json = serde_json::to_string(&events).expect("serialize");
        let back: Vec<LlmEvent> = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(events, back);
    }

    #[test]
    fn finish_reason_maps_chat_strings() {
        assert_eq!(
            FinishReason::from_chat(Some("stop"), false),
            FinishReason::Stop
        );
        assert_eq!(
            FinishReason::from_chat(Some("tool_calls"), false),
            FinishReason::ToolCalls
        );
        assert_eq!(
            FinishReason::from_chat(None, true),
            FinishReason::ToolCalls,
            "tool-call fragments without a terminal reason imply ToolCalls"
        );
        assert_eq!(FinishReason::from_chat(None, false), FinishReason::Stop);
        assert_eq!(
            FinishReason::from_chat(Some("weird"), false),
            FinishReason::Other("weird".to_owned())
        );
    }
}
