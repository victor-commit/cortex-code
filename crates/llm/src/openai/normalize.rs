//! Normalization of an OpenAI-compatible Chat-completions SSE stream into [`LlmEvent`]s.
//!
//! This is the reference path used for OpenRouter. It consumes the `data:` payloads of a streamed
//! `chat.completion.chunk` sequence and produces an ordered [`LlmEvent`] stream: text and reasoning
//! deltas in generation order, then any tool calls (with their streamed argument fragments
//! assembled into one complete JSON string), then a usage summary, and finally exactly one
//! [`LlmEvent::Done`]. It is wire-only and network-free, so the same code path serves both the live
//! transport and the offline cassette tests.

use std::collections::VecDeque;
use std::pin::Pin;

use futures_util::{Stream, StreamExt};

use crate::error::LlmError;
use crate::event::{FinishReason, LlmEvent, ToolCall, Usage};

use super::wire::ChatChunk;

/// The terminal sentinel an OpenAI-compatible stream sends to mark completion.
const DONE_SENTINEL: &str = "[DONE]";

/// A boxed stream of SSE `data:` payloads (or transport errors), the input to normalization.
pub type FrameStream = Pin<Box<dyn Stream<Item = Result<String, LlmError>> + Send>>;

/// Normalize an OpenAI-compatible Chat SSE `data:` frame stream into ordered [`LlmEvent`]s.
///
/// The returned stream always terminates with a single [`LlmEvent::Done`]. A transport error in
/// `frames` surfaces as an [`LlmEvent::Error`] immediately followed by the terminal `Done`.
pub fn normalize_chat_sse(frames: FrameStream) -> impl Stream<Item = LlmEvent> + Send {
    futures_util::stream::unfold(NormalizeState::new(frames), |mut state| async move {
        loop {
            if let Some(event) = state.queue.pop_front() {
                return Some((event, state));
            }
            if state.terminated {
                return None;
            }
            match state.next_step().await {
                Step::Emitted => continue,
                Step::Exhausted => return None,
            }
        }
    })
}

/// Outcome of pulling one frame: either events were queued, or the stream is fully drained.
enum Step {
    Emitted,
    Exhausted,
}

/// Streaming normalization state machine over the frame stream.
struct NormalizeState {
    frames: FrameStream,
    accumulator: ChatAccumulator,
    queue: VecDeque<LlmEvent>,
    frames_done: bool,
    terminated: bool,
}

impl NormalizeState {
    fn new(frames: FrameStream) -> Self {
        Self {
            frames,
            accumulator: ChatAccumulator::default(),
            queue: VecDeque::new(),
            frames_done: false,
            terminated: false,
        }
    }

    async fn next_step(&mut self) -> Step {
        if self.frames_done {
            self.accumulator.finalize(&mut self.queue);
            self.terminated = true;
            return Step::Emitted;
        }
        match self.frames.next().await {
            Some(Ok(data)) => {
                if data.trim() == DONE_SENTINEL {
                    self.accumulator.finalize(&mut self.queue);
                    self.terminated = true;
                } else {
                    self.accumulator.push_frame(&data, &mut self.queue);
                }
                Step::Emitted
            }
            Some(Err(err)) => {
                // Forward the bare detail (not the rendered `Display`, which prepends the class
                // prefix) so a consumer re-wrapping by `kind` renders the prefix exactly once.
                self.queue.push_back(LlmEvent::Error {
                    message: err.detail().to_owned(),
                    kind: err.kind().to_owned(),
                });
                self.queue.push_back(LlmEvent::Done {
                    reason: FinishReason::Error,
                });
                self.terminated = true;
                Step::Emitted
            }
            None => {
                if self.queue.is_empty() && !self.terminated {
                    self.frames_done = true;
                    Step::Emitted
                } else {
                    Step::Exhausted
                }
            }
        }
    }
}

/// In-progress assembly of a single tool call across streamed fragments.
#[derive(Default)]
struct ToolAccumulator {
    id: String,
    name: String,
    arguments: String,
}

/// Accumulates per-frame deltas of a Chat-completions stream into ordered [`LlmEvent`]s.
#[derive(Default)]
struct ChatAccumulator {
    tools: Vec<ToolAccumulator>,
    usage: Option<Usage>,
    finish_reason: Option<String>,
    saw_tool_calls: bool,
}

impl ChatAccumulator {
    /// Parse one Chat chunk and queue the events it produces (text/reasoning deltas immediately;
    /// tool-call fragments are buffered for assembly at [`ChatAccumulator::finalize`]).
    fn push_frame(&mut self, data: &str, queue: &mut VecDeque<LlmEvent>) {
        let Ok(chunk) = serde_json::from_str::<ChatChunk>(data) else {
            // Unparseable non-`[DONE]` frames are almost always keep-alives that slipped past the
            // SSE comment filter; ignore them rather than aborting a healthy turn.
            return;
        };

        if let Some(error) = chunk.error {
            queue.push_back(LlmEvent::Error {
                message: redaction::redact(&error.message()),
                kind: "upstream".to_owned(),
            });
            self.finish_reason.get_or_insert_with(|| "error".to_owned());
            return;
        }

        if let Some(usage) = chunk.usage {
            self.usage = Some(usage.into_usage());
        }

        for choice in chunk.choices {
            let reasoning = choice.delta.reasoning_text();
            let reasoning_metadata = choice.delta.reasoning_metadata();
            if let Some(text) = choice.delta.content
                && !text.is_empty()
            {
                queue.push_back(LlmEvent::TextDelta { text });
            }
            if reasoning.as_ref().is_some_and(|text| !text.is_empty())
                || reasoning_metadata.is_some()
            {
                queue.push_back(LlmEvent::Reasoning {
                    text: reasoning.unwrap_or_default(),
                    metadata: reasoning_metadata,
                });
            }
            for delta in choice.delta.tool_calls {
                self.saw_tool_calls = true;
                self.accumulate_tool_call(delta);
            }
            if let Some(reason) = choice.finish_reason {
                self.finish_reason = Some(reason);
            }
        }
    }

    fn accumulate_tool_call(&mut self, delta: super::wire::ToolCallDelta) {
        let index = delta.index;
        while self.tools.len() <= index {
            self.tools.push(ToolAccumulator::default());
        }
        let slot = &mut self.tools[index];
        if let Some(id) = delta.id
            && !id.is_empty()
        {
            slot.id = id;
        }
        if let Some(function) = delta.function {
            if let Some(name) = function.name
                && !name.is_empty()
            {
                slot.name = name;
            }
            if let Some(arguments) = function.arguments {
                slot.arguments.push_str(&arguments);
            }
        }
    }

    /// Emit the buffered tool calls, the usage summary, and the terminal [`LlmEvent::Done`].
    fn finalize(&mut self, queue: &mut VecDeque<LlmEvent>) {
        for tool in self.tools.drain(..) {
            queue.push_back(LlmEvent::ToolCall(ToolCall {
                id: tool.id,
                name: tool.name,
                arguments: tool.arguments,
            }));
        }
        if let Some(usage) = self.usage.take() {
            queue.push_back(LlmEvent::Usage(usage));
        }
        queue.push_back(LlmEvent::Done {
            reason: FinishReason::from_chat(self.finish_reason.as_deref(), self.saw_tool_calls),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frames(datas: Vec<&'static str>) -> FrameStream {
        let iter = datas
            .into_iter()
            .map(|d| Ok::<String, LlmError>(d.to_owned()));
        Box::pin(futures_util::stream::iter(iter))
    }

    async fn collect(stream: impl Stream<Item = LlmEvent>) -> Vec<LlmEvent> {
        let stream = Box::pin(stream);
        stream.collect().await
    }

    #[tokio::test]
    async fn text_deltas_concatenate_and_end_in_done() {
        let events = collect(normalize_chat_sse(frames(vec![
            r#"{"choices":[{"delta":{"role":"assistant","content":"Hel"},"finish_reason":null}]}"#,
            r#"{"choices":[{"delta":{"content":"lo!"},"finish_reason":"stop"}]}"#,
            "[DONE]",
        ])))
        .await;

        let text: String = events
            .iter()
            .filter_map(|e| match e {
                LlmEvent::TextDelta { text } => Some(text.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(text, "Hello!");
        assert_eq!(
            events.last(),
            Some(&LlmEvent::Done {
                reason: FinishReason::Stop
            })
        );
    }

    #[tokio::test]
    async fn reasoning_details_emit_reasoning_event_even_when_text_is_absent() {
        let events = collect(normalize_chat_sse(frames(vec![
            r#"{"choices":[{"delta":{"reasoning_details":[{"type":"reasoning.encrypted","data":"cipher-1","id":"rs_1"}]}}]}"#,
            r#"{"choices":[{"delta":{"content":"FINAL:378"},"finish_reason":"stop"}]}"#,
            "[DONE]",
        ])))
        .await;

        let reasoning = events
            .iter()
            .find_map(|event| match event {
                LlmEvent::Reasoning { text, metadata } => Some((text, metadata)),
                _ => None,
            })
            .expect("a reasoning event");
        assert_eq!(reasoning.0, "");
        let metadata = reasoning.1.as_ref().expect("reasoning metadata");
        assert_eq!(metadata["provider"], "openai");
        assert_eq!(metadata["signature"], "cipher-1");
        assert!(metadata["details"].is_array());
    }

    #[tokio::test]
    async fn tool_call_argument_fragments_are_assembled() {
        let events = collect(normalize_chat_sse(frames(vec![
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_a","type":"function","function":{"name":"get_weather","arguments":""}}]}}]}"#,
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\"city\":"}}]}}]}"#,
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"\"SF\"}"}}]}}]}"#,
            r#"{"choices":[{"delta":{},"finish_reason":"tool_calls"}]}"#,
            "[DONE]",
        ])))
        .await;

        let tool_call = events
            .iter()
            .find_map(|e| match e {
                LlmEvent::ToolCall(call) => Some(call),
                _ => None,
            })
            .expect("a tool call");
        assert_eq!(tool_call.id, "call_a");
        assert_eq!(tool_call.name, "get_weather");
        assert_eq!(tool_call.arguments, r#"{"city":"SF"}"#);
        assert_eq!(
            events.last(),
            Some(&LlmEvent::Done {
                reason: FinishReason::ToolCalls
            })
        );
    }

    #[tokio::test]
    async fn missing_done_sentinel_still_finalizes_with_done() {
        let events = collect(normalize_chat_sse(frames(vec![
            r#"{"choices":[{"delta":{"content":"hi"},"finish_reason":"stop"}]}"#,
        ])))
        .await;
        assert_eq!(
            events.last(),
            Some(&LlmEvent::Done {
                reason: FinishReason::Stop
            })
        );
    }

    #[tokio::test]
    async fn mid_stream_transport_error_surfaces_then_done() {
        let iter = vec![
            Ok::<String, LlmError>(r#"{"choices":[{"delta":{"content":"partial"}}]}"#.to_owned()),
            Err(LlmError::transport("connection reset")),
        ];
        let stream: FrameStream = Box::pin(futures_util::stream::iter(iter));
        let events = collect(normalize_chat_sse(stream)).await;

        assert!(matches!(events.first(), Some(LlmEvent::TextDelta { .. })));
        assert!(events.iter().any(|e| matches!(e, LlmEvent::Error { .. })));
        assert_eq!(
            events.last(),
            Some(&LlmEvent::Done {
                reason: FinishReason::Error
            })
        );
    }

    #[tokio::test]
    async fn usage_is_emitted_before_done() {
        let events = collect(normalize_chat_sse(frames(vec![
            r#"{"choices":[{"delta":{"content":"hi"},"finish_reason":"stop"}]}"#,
            r#"{"choices":[],"usage":{"prompt_tokens":5,"completion_tokens":2,"total_tokens":7}}"#,
            "[DONE]",
        ])))
        .await;

        let usage_pos = events
            .iter()
            .position(|e| matches!(e, LlmEvent::Usage(_)))
            .expect("a usage event");
        let done_pos = events
            .iter()
            .position(|e| matches!(e, LlmEvent::Done { .. }))
            .expect("a done event");
        assert!(usage_pos < done_pos, "usage must precede done");
        match &events[usage_pos] {
            LlmEvent::Usage(usage) => {
                assert_eq!(usage.input_tokens, 5);
                assert_eq!(usage.output_tokens, 2);
                assert_eq!(usage.total_tokens, 7);
            }
            _ => unreachable!(),
        }
    }
}
