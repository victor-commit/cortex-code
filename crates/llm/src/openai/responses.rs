//! Native OpenAI Responses-protocol adapter.
//!
//! This adapter complements the Chat-completions path with OpenAI's native `/responses` SSE
//! protocol. It normalizes streamed responses into the canonical `LlmEvent` vocabulary and keeps
//! the same fail-closed guard + credential behavior as the Chat adapter.

use std::collections::BTreeMap;
use std::collections::VecDeque;
use std::fmt;

use futures_util::Stream;
use futures_util::StreamExt;
use futures_util::future::BoxFuture;
use redaction::{PLACEHOLDER, guard_outbound};
use serde_json::Value as JsonValue;

use crate::error::LlmError;
use crate::event::{FinishReason, LlmEvent, ToolCall, Usage};
use crate::provider::{LlmStream, Provider};
use crate::request::{LlmRequest, Role};
use crate::transport::{HttpTransport, TransportConfig, sse_data_frames};

/// The outbound operation named in a `net-disabled` refusal raised by the adapter.
const OUTBOUND_OPERATION: &str = "LLM streaming request";

/// Configuration for an [`OpenAiResponses`] provider.
#[derive(Clone)]
pub struct OpenAiResponsesConfig {
    /// The API base URL (for example `https://api.openai.com/v1`).
    pub base_url: String,
    /// The bearer API key.
    pub api_key: String,
    /// A human label for diagnostics.
    pub provider_name: String,
    /// Extra request headers.
    pub extra_headers: Vec<(String, String)>,
    /// Transport tuning.
    pub transport: TransportConfig,
}

impl fmt::Debug for OpenAiResponsesConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OpenAiResponsesConfig")
            .field("base_url", &self.base_url)
            .field("api_key", &PLACEHOLDER)
            .field("provider_name", &self.provider_name)
            .field("extra_headers", &self.extra_headers)
            .field("transport", &self.transport)
            .finish()
    }
}

/// Native OpenAI Responses provider.
#[derive(Clone)]
pub struct OpenAiResponses {
    transport: HttpTransport,
    base_url: String,
    api_key: String,
    provider_name: String,
    extra_headers: Vec<(String, String)>,
}

impl OpenAiResponses {
    /// Build a provider from configuration.
    pub fn new(config: OpenAiResponsesConfig) -> Result<Self, LlmError> {
        let transport = HttpTransport::new(config.transport)?;
        Ok(Self {
            transport,
            base_url: config.base_url,
            api_key: config.api_key,
            provider_name: config.provider_name,
            extra_headers: config.extra_headers,
        })
    }

    fn responses_url(&self) -> String {
        format!("{}/responses", self.base_url.trim_end_matches('/'))
    }
}

impl fmt::Debug for OpenAiResponses {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OpenAiResponses")
            .field("transport", &self.transport)
            .field("base_url", &self.base_url)
            .field("api_key", &PLACEHOLDER)
            .field("provider_name", &self.provider_name)
            .field("extra_headers", &self.extra_headers)
            .finish()
    }
}

impl Provider for OpenAiResponses {
    fn stream<'a>(&'a self, request: LlmRequest) -> BoxFuture<'a, Result<LlmStream, LlmError>> {
        Box::pin(async move {
            guard_outbound(OUTBOUND_OPERATION).map_err(LlmError::from_diagnostic)?;

            if self.api_key.is_empty() {
                return Err(LlmError::no_credential(&self.provider_name));
            }

            let body = build_responses_request(&request, &self.provider_name)?;
            let mut headers = self.extra_headers.clone();
            headers.extend(request.extra_headers.iter().cloned());
            let bytes = self
                .transport
                .open_post_sse(&self.responses_url(), &self.api_key, &headers, body)
                .await?;
            let frames = Box::pin(sse_data_frames(bytes, self.transport.config().idle_timeout));
            Ok(Box::pin(normalize_responses_sse(frames)) as LlmStream)
        })
    }
}

fn build_responses_request(request: &LlmRequest, provider_name: &str) -> Result<String, LlmError> {
    let model = normalize_model_id(&request.model, provider_name);
    let mut input = Vec::new();
    for message in &request.messages {
        match message.role {
            Role::System | Role::User | Role::Assistant => {
                if !message.content.is_empty() {
                    input.push(serde_json::json!({
                        "role": role_name(message.role),
                        "content": message.content,
                    }));
                }
                if message.role == Role::Assistant {
                    for call in &message.tool_calls {
                        input.push(serde_json::json!({
                            "type": "function_call",
                            "call_id": call.id,
                            "name": call.name,
                            "arguments": call.arguments,
                        }));
                    }
                }
            }
            Role::Tool => {
                input.push(serde_json::json!({
                    "type": "function_call_output",
                    "call_id": message
                        .tool_call_id
                        .clone()
                        .unwrap_or_else(|| "tool_call".to_owned()),
                    "output": message.content,
                }));
            }
        }
    }

    let mut value = serde_json::json!({
        "model": model,
        "input": input,
        "stream": true,
    });
    if let Some(max_tokens) = request.max_tokens {
        value["max_output_tokens"] = serde_json::json!(max_tokens);
    }
    if let Some(temperature) = request.temperature {
        value["temperature"] = serde_json::json!(temperature);
    }
    if !request.tools.is_empty() {
        value["tools"] = serde_json::Value::Array(
            request
                .tools
                .iter()
                .map(|tool| {
                    serde_json::json!({
                        "type": "function",
                        "name": tool.name,
                        "description": tool.description,
                        "parameters": tool.parameters,
                    })
                })
                .collect(),
        );
    }
    if let Some(map) = value.as_object_mut() {
        for (name, param) in &request.extra_params {
            if !is_reserved_responses_param(name) {
                map.insert(name.clone(), param.clone());
            }
        }
    }
    serde_json::to_string(&value)
        .map_err(|err| LlmError::transport(format!("failed to encode request: {err}")))
}

fn is_reserved_responses_param(name: &str) -> bool {
    matches!(
        name,
        "model" | "input" | "tools" | "stream" | "max_output_tokens" | "temperature"
    )
}

fn normalize_model_id(model_id: &str, provider_name: &str) -> String {
    let Some((prefix, rest)) = model_id.split_once('/') else {
        return model_id.to_owned();
    };
    if prefix.eq_ignore_ascii_case(provider_name) {
        rest.to_owned()
    } else {
        model_id.to_owned()
    }
}

fn role_name(role: Role) -> &'static str {
    match role {
        Role::System => "system",
        Role::User | Role::Tool => "user",
        Role::Assistant => "assistant",
    }
}

type FrameStream = std::pin::Pin<Box<dyn Stream<Item = Result<String, LlmError>> + Send>>;

fn normalize_responses_sse(frames: FrameStream) -> impl Stream<Item = LlmEvent> + Send {
    futures_util::stream::unfold(NormalizeState::new(frames), |mut state| async move {
        loop {
            if let Some(event) = state.queue.pop_front() {
                return Some((event, state));
            }
            if state.terminated {
                return None;
            }
            state.next().await;
        }
    })
}

#[derive(Default)]
struct ToolAccumulator {
    id: String,
    name: String,
    arguments: String,
}

struct NormalizeState {
    frames: FrameStream,
    queue: VecDeque<LlmEvent>,
    tools: Vec<ToolAccumulator>,
    tool_index: BTreeMap<String, usize>,
    usage: Option<Usage>,
    saw_tool_calls: bool,
    terminated: bool,
}

impl NormalizeState {
    fn new(frames: FrameStream) -> Self {
        Self {
            frames,
            queue: VecDeque::new(),
            tools: Vec::new(),
            tool_index: BTreeMap::new(),
            usage: None,
            saw_tool_calls: false,
            terminated: false,
        }
    }

    async fn next(&mut self) {
        let Some(frame) = self.frames.next().await else {
            self.finalize(FinishReason::from_chat(None, self.saw_tool_calls));
            return;
        };
        match frame {
            Err(err) => {
                self.queue.push_back(LlmEvent::Error {
                    message: err.detail().to_owned(),
                    kind: err.kind().to_owned(),
                });
                self.finalize(FinishReason::Error);
            }
            Ok(data) if data.trim() == "[DONE]" => {
                self.finalize(FinishReason::from_chat(None, self.saw_tool_calls));
            }
            Ok(data) => self.consume_frame(&data),
        }
    }

    fn consume_frame(&mut self, data: &str) {
        let Ok(value) = serde_json::from_str::<JsonValue>(data) else {
            return;
        };
        if let Some(error) = value.get("error") {
            let message = error
                .get("message")
                .and_then(JsonValue::as_str)
                .unwrap_or("unspecified upstream error");
            self.queue.push_back(LlmEvent::Error {
                message: redaction::redact(message),
                kind: "upstream".to_owned(),
            });
            self.finalize(FinishReason::Error);
            return;
        }
        let event_type = value
            .get("type")
            .and_then(JsonValue::as_str)
            .unwrap_or_default();
        match event_type {
            "response.output_text.delta" => {
                if let Some(delta) = value.get("delta").and_then(JsonValue::as_str)
                    && !delta.is_empty()
                {
                    self.queue.push_back(LlmEvent::TextDelta {
                        text: delta.to_owned(),
                    });
                }
            }
            "response.output_text.done" => {
                if let Some(text) = value.get("text").and_then(JsonValue::as_str)
                    && !text.is_empty()
                {
                    self.queue.push_back(LlmEvent::TextDelta {
                        text: text.to_owned(),
                    });
                }
            }
            "response.function_call_arguments.delta" => {
                self.saw_tool_calls = true;
                let key = value
                    .get("item_id")
                    .or_else(|| value.get("output_item_id"))
                    .and_then(JsonValue::as_str)
                    .unwrap_or_default();
                let delta = value
                    .get("delta")
                    .and_then(JsonValue::as_str)
                    .unwrap_or_default();
                if !key.is_empty() && !delta.is_empty() {
                    let slot = self.tool_slot(key);
                    slot.arguments.push_str(delta);
                }
            }
            "response.function_call_arguments.done" => {
                self.saw_tool_calls = true;
                let key = value
                    .get("item_id")
                    .or_else(|| value.get("output_item_id"))
                    .and_then(JsonValue::as_str)
                    .unwrap_or_default();
                if !key.is_empty()
                    && let Some(arguments) = value.get("arguments").and_then(JsonValue::as_str)
                {
                    self.tool_slot(key).arguments = arguments.to_owned();
                }
            }
            "response.output_item.added" | "response.output_item.done" => {
                self.capture_output_item(value.get("item"));
            }
            "response.completed" => {
                if let Some(usage) = parse_usage(value.get("response").and_then(|r| r.get("usage")))
                {
                    self.usage = Some(usage);
                }
            }
            other if other.contains("reasoning") => {
                if let Some(delta) = value
                    .get("delta")
                    .or_else(|| value.get("text"))
                    .and_then(JsonValue::as_str)
                    && !delta.is_empty()
                {
                    self.queue.push_back(LlmEvent::Reasoning {
                        text: delta.to_owned(),
                    });
                }
            }
            _ => {
                if let Some(usage) = parse_usage(value.get("usage")) {
                    self.usage = Some(usage);
                }
                if let Some(outputs) = value
                    .get("response")
                    .and_then(|response| response.get("output"))
                    .and_then(JsonValue::as_array)
                {
                    for item in outputs {
                        self.capture_output_item(Some(item));
                    }
                }
            }
        }
    }

    fn capture_output_item(&mut self, item: Option<&JsonValue>) {
        let Some(item) = item else {
            return;
        };
        if item.get("type").and_then(JsonValue::as_str) != Some("function_call") {
            return;
        }
        self.saw_tool_calls = true;
        let key = item
            .get("id")
            .or_else(|| item.get("call_id"))
            .and_then(JsonValue::as_str)
            .unwrap_or_default();
        if key.is_empty() {
            return;
        }
        let slot = self.tool_slot(key);
        if let Some(call_id) = item.get("call_id").and_then(JsonValue::as_str)
            && !call_id.is_empty()
        {
            slot.id = call_id.to_owned();
        } else if slot.id.is_empty() {
            slot.id = item
                .get("id")
                .and_then(JsonValue::as_str)
                .unwrap_or(key)
                .to_owned();
        }
        if slot.name.is_empty()
            && let Some(name) = item.get("name").and_then(JsonValue::as_str)
        {
            slot.name = name.to_owned();
        }
        if let Some(arguments) = item.get("arguments").and_then(JsonValue::as_str)
            && !arguments.is_empty()
        {
            slot.arguments = arguments.to_owned();
        }
    }

    fn tool_slot(&mut self, key: &str) -> &mut ToolAccumulator {
        if let Some(index) = self.tool_index.get(key).copied() {
            return self
                .tools
                .get_mut(index)
                .expect("tool index should point into the tools vector");
        }
        let index = self.tools.len();
        self.tools.push(ToolAccumulator {
            id: key.to_owned(),
            ..ToolAccumulator::default()
        });
        self.tool_index.insert(key.to_owned(), index);
        self.tools
            .get_mut(index)
            .expect("just-pushed tool accumulator")
    }

    fn finalize(&mut self, reason: FinishReason) {
        for tool in self.tools.drain(..) {
            self.queue.push_back(LlmEvent::ToolCall(ToolCall {
                id: tool.id,
                name: tool.name,
                arguments: tool.arguments,
            }));
        }
        self.tool_index.clear();
        if let Some(usage) = self.usage.take() {
            self.queue.push_back(LlmEvent::Usage(usage));
        }
        self.queue.push_back(LlmEvent::Done { reason });
        self.terminated = true;
    }
}

fn parse_usage(value: Option<&JsonValue>) -> Option<Usage> {
    let usage = value?;
    let input_tokens = usage
        .get("input_tokens")
        .or_else(|| usage.get("prompt_tokens"))
        .and_then(JsonValue::as_u64)? as u32;
    let output_tokens = usage
        .get("output_tokens")
        .or_else(|| usage.get("completion_tokens"))
        .and_then(JsonValue::as_u64)
        .unwrap_or(0) as u32;
    let total_tokens = usage
        .get("total_tokens")
        .and_then(JsonValue::as_u64)
        .map(|value| value as u32)
        .unwrap_or_else(|| input_tokens.saturating_add(output_tokens));
    let cache_creation_input_tokens = usage
        .get("cache_creation_input_tokens")
        .and_then(JsonValue::as_u64)
        .and_then(|value| u32::try_from(value).ok());
    let cache_read_input_tokens = usage
        .get("cache_read_input_tokens")
        .and_then(JsonValue::as_u64)
        .and_then(|value| u32::try_from(value).ok())
        .or_else(|| {
            usage
                .get("input_tokens_details")
                .and_then(|details| details.get("cached_tokens"))
                .and_then(JsonValue::as_u64)
                .and_then(|value| u32::try_from(value).ok())
        });
    let cost = usage.get("cost").and_then(JsonValue::as_f64);
    Some(Usage {
        input_tokens,
        output_tokens,
        total_tokens,
        cache_creation_input_tokens,
        cache_read_input_tokens,
        cost,
    })
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use bytes::Bytes;

    use super::*;
    use crate::transport::sse_data_frames;

    #[test]
    fn request_uses_function_call_output_for_tool_messages() {
        let request = LlmRequest::new("openai/gpt-4.1")
            .message(crate::request::Message::assistant("working"))
            .message(crate::request::Message::tool_result("call_1", "ok"));
        let encoded = build_responses_request(&request, "openai").expect("request");
        let json: JsonValue = serde_json::from_str(&encoded).expect("json");
        assert_eq!(json["model"], "gpt-4.1");
        assert!(
            json["input"]
                .as_array()
                .expect("input array")
                .iter()
                .any(
                    |entry| entry["type"] == "function_call_output" && entry["call_id"] == "call_1"
                )
        );
    }

    #[tokio::test]
    async fn normalizer_emits_tool_calls_and_usage() {
        let stream = futures_util::stream::iter(vec![
            Ok::<Bytes, LlmError>(Bytes::from(
                "data: {\"type\":\"response.output_item.added\",\"item\":{\"type\":\"function_call\",\"id\":\"fc_1\",\"call_id\":\"call_1\",\"name\":\"read\"}}\n\n",
            )),
            Ok::<Bytes, LlmError>(Bytes::from(
                "data: {\"type\":\"response.function_call_arguments.delta\",\"item_id\":\"fc_1\",\"delta\":\"{\\\"path\\\":\"}\n\n",
            )),
            Ok::<Bytes, LlmError>(Bytes::from(
                "data: {\"type\":\"response.function_call_arguments.delta\",\"item_id\":\"fc_1\",\"delta\":\"\\\"Cargo.toml\\\"}\"}\n\n",
            )),
            Ok::<Bytes, LlmError>(Bytes::from(
                "data: {\"type\":\"response.completed\",\"response\":{\"usage\":{\"input_tokens\":4,\"output_tokens\":2,\"total_tokens\":6,\"input_tokens_details\":{\"cached_tokens\":3}}}}\n\n",
            )),
            Ok::<Bytes, LlmError>(Bytes::from("data: [DONE]\n\n")),
        ]);
        let bytes: crate::transport::ByteStream = Box::pin(stream);
        let frames = Box::pin(sse_data_frames(bytes, Duration::from_secs(1)));
        let events: Vec<LlmEvent> = normalize_responses_sse(frames).collect().await;
        assert!(
            events.iter().any(
                |event| matches!(event, LlmEvent::ToolCall(call) if call.id == "call_1" && call.name == "read")
            )
        );
        assert!(events.iter().any(|event| matches!(
            event,
            LlmEvent::Usage(usage)
                if usage.total_tokens == 6
                    && usage.cache_read_input_tokens == Some(3)
        )));
        assert!(matches!(events.last(), Some(LlmEvent::Done { .. })));
    }
}
