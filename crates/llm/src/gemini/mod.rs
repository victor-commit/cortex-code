//! Native Google Gemini adapter.
//!
//! This adapter speaks Gemini's native `streamGenerateContent` protocol and normalizes text,
//! thought/reasoning, and function-call parts into the canonical `LlmEvent` stream.

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

/// Default Gemini API base.
pub const GEMINI_API_BASE: &str = "https://generativelanguage.googleapis.com/v1beta";
const OUTBOUND_OPERATION: &str = "LLM streaming request";

/// Configuration for [`GeminiProvider`].
#[derive(Clone)]
pub struct GeminiConfig {
    pub base_url: String,
    pub api_key: String,
    pub provider_name: String,
    pub extra_headers: Vec<(String, String)>,
    pub transport: TransportConfig,
}

impl fmt::Debug for GeminiConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GeminiConfig")
            .field("base_url", &self.base_url)
            .field("api_key", &PLACEHOLDER)
            .field("provider_name", &self.provider_name)
            .field("extra_headers", &self.extra_headers)
            .field("transport", &self.transport)
            .finish()
    }
}

/// Native Gemini provider.
#[derive(Clone)]
pub struct GeminiProvider {
    transport: HttpTransport,
    base_url: String,
    api_key: String,
    provider_name: String,
    extra_headers: Vec<(String, String)>,
}

impl GeminiProvider {
    pub fn new(config: GeminiConfig) -> Result<Self, LlmError> {
        let transport = HttpTransport::new(config.transport)?;
        Ok(Self {
            transport,
            base_url: config.base_url,
            api_key: config.api_key,
            provider_name: config.provider_name,
            extra_headers: config.extra_headers,
        })
    }

    fn stream_url(&self, model_id: &str) -> String {
        let model = normalize_model_id(model_id);
        let model_path = if model.starts_with("models/") {
            model
        } else {
            format!("models/{model}")
        };
        format!(
            "{}/{model_path}:streamGenerateContent?alt=sse",
            self.base_url.trim_end_matches('/')
        )
    }
}

impl fmt::Debug for GeminiProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GeminiProvider")
            .field("transport", &self.transport)
            .field("base_url", &self.base_url)
            .field("api_key", &PLACEHOLDER)
            .field("provider_name", &self.provider_name)
            .field("extra_headers", &self.extra_headers)
            .finish()
    }
}

impl Provider for GeminiProvider {
    fn stream<'a>(&'a self, request: LlmRequest) -> BoxFuture<'a, Result<LlmStream, LlmError>> {
        Box::pin(async move {
            guard_outbound(OUTBOUND_OPERATION).map_err(LlmError::from_diagnostic)?;

            if self.api_key.is_empty() {
                return Err(LlmError::no_credential(&self.provider_name));
            }

            let body = build_gemini_request(&request)?;
            let mut headers = vec![("x-goog-api-key".to_owned(), self.api_key.clone())];
            headers.extend(self.extra_headers.clone());
            headers.extend(request.extra_headers.iter().cloned());
            let bytes = self
                .transport
                .open_post_sse_with_headers(&self.stream_url(&request.model), &headers, body)
                .await?;
            let frames = Box::pin(sse_data_frames(bytes, self.transport.config().idle_timeout));
            Ok(Box::pin(normalize_gemini_sse(frames)) as LlmStream)
        })
    }
}

fn build_gemini_request(request: &LlmRequest) -> Result<String, LlmError> {
    let mut contents = Vec::new();
    let mut system_parts = Vec::new();
    for message in &request.messages {
        match message.role {
            Role::System => {
                if !message.content.is_empty() {
                    system_parts.push(serde_json::json!({ "text": message.content }));
                }
            }
            Role::User => {
                contents.push(serde_json::json!({
                    "role": "user",
                    "parts": [{ "text": message.content }],
                }));
            }
            Role::Assistant => {
                let mut parts = Vec::new();
                if !message.content.is_empty() {
                    parts.push(serde_json::json!({ "text": message.content }));
                }
                for call in &message.tool_calls {
                    parts.push(serde_json::json!({
                        "functionCall": {
                            "name": call.name,
                            "args": parse_json_arguments(&call.arguments),
                        }
                    }));
                }
                contents.push(serde_json::json!({
                    "role": "model",
                    "parts": parts,
                }));
            }
            Role::Tool => {
                contents.push(serde_json::json!({
                    "role": "user",
                    "parts": [{
                        "functionResponse": {
                            "name": message
                                .tool_call_id
                                .clone()
                                .unwrap_or_else(|| "tool_result".to_owned()),
                            "response": { "content": message.content },
                        }
                    }],
                }));
            }
        }
    }
    let mut body = serde_json::json!({ "contents": contents });
    if !system_parts.is_empty() {
        body["systemInstruction"] = serde_json::json!({ "parts": system_parts });
    }
    if !request.tools.is_empty() {
        body["tools"] = serde_json::json!([{
            "functionDeclarations": request.tools.iter().map(|tool| serde_json::json!({
                "name": tool.name,
                "description": tool.description,
                "parameters": tool.parameters,
            })).collect::<Vec<_>>()
        }]);
    }
    if request.max_tokens.is_some() || request.temperature.is_some() {
        let mut config = serde_json::Map::new();
        if let Some(max_tokens) = request.max_tokens {
            config.insert("maxOutputTokens".to_owned(), serde_json::json!(max_tokens));
        }
        if let Some(temperature) = request.temperature {
            config.insert("temperature".to_owned(), serde_json::json!(temperature));
        }
        body["generationConfig"] = JsonValue::Object(config);
    }
    if let Some(map) = body.as_object_mut() {
        for (name, param) in &request.extra_params {
            if !is_reserved_gemini_param(name) {
                map.insert(name.clone(), param.clone());
            }
        }
    }
    serde_json::to_string(&body)
        .map_err(|err| LlmError::transport(format!("failed to encode request: {err}")))
}

fn is_reserved_gemini_param(name: &str) -> bool {
    matches!(
        name,
        "contents" | "systemInstruction" | "tools" | "generationConfig"
    )
}

fn normalize_model_id(model_id: &str) -> String {
    let Some((prefix, rest)) = model_id.split_once('/') else {
        return model_id.to_owned();
    };
    if prefix.eq_ignore_ascii_case("google") || prefix.eq_ignore_ascii_case("gemini") {
        rest.to_owned()
    } else {
        model_id.to_owned()
    }
}

fn parse_json_arguments(arguments: &str) -> JsonValue {
    serde_json::from_str(arguments).unwrap_or_else(|_| serde_json::json!({ "raw": arguments }))
}

type FrameStream = std::pin::Pin<Box<dyn Stream<Item = Result<String, LlmError>> + Send>>;

fn normalize_gemini_sse(frames: FrameStream) -> impl Stream<Item = LlmEvent> + Send {
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

struct NormalizeState {
    frames: FrameStream,
    queue: VecDeque<LlmEvent>,
    tool_calls: BTreeMap<String, ToolCall>,
    usage: Option<Usage>,
    finish_reason: Option<FinishReason>,
    saw_tool_calls: bool,
    terminated: bool,
}

impl NormalizeState {
    fn new(frames: FrameStream) -> Self {
        Self {
            frames,
            queue: VecDeque::new(),
            tool_calls: BTreeMap::new(),
            usage: None,
            finish_reason: None,
            saw_tool_calls: false,
            terminated: false,
        }
    }

    async fn next(&mut self) {
        let Some(frame) = self.frames.next().await else {
            self.finalize();
            return;
        };
        match frame {
            Err(err) => {
                self.queue.push_back(LlmEvent::Error {
                    message: err.detail().to_owned(),
                    kind: err.kind().to_owned(),
                });
                self.queue.push_back(LlmEvent::Done {
                    reason: FinishReason::Error,
                });
                self.terminated = true;
            }
            Ok(data) if data.trim() == "[DONE]" => self.finalize(),
            Ok(data) => self.consume(&data),
        }
    }

    fn consume(&mut self, data: &str) {
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
            self.queue.push_back(LlmEvent::Done {
                reason: FinishReason::Error,
            });
            self.terminated = true;
            return;
        }

        if let Some(usage) = parse_usage(value.get("usageMetadata")) {
            self.usage = Some(usage);
        }

        let Some(candidates) = value.get("candidates").and_then(JsonValue::as_array) else {
            return;
        };
        for candidate in candidates {
            if let Some(reason) = candidate
                .get("finishReason")
                .and_then(JsonValue::as_str)
                .map(map_finish_reason)
            {
                self.finish_reason = Some(reason);
            }

            let Some(parts) = candidate
                .get("content")
                .and_then(|content| content.get("parts"))
                .and_then(JsonValue::as_array)
            else {
                continue;
            };
            for part in parts {
                if let Some(text) = part.get("text").and_then(JsonValue::as_str) {
                    if part.get("thought").and_then(JsonValue::as_bool) == Some(true) {
                        if !text.is_empty() {
                            self.queue.push_back(LlmEvent::Reasoning {
                                text: text.to_owned(),
                            });
                        }
                    } else if !text.is_empty() {
                        self.queue.push_back(LlmEvent::TextDelta {
                            text: text.to_owned(),
                        });
                    }
                }
                if let Some(function_call) = part.get("functionCall") {
                    self.saw_tool_calls = true;
                    let name = function_call
                        .get("name")
                        .and_then(JsonValue::as_str)
                        .unwrap_or_default();
                    if name.is_empty() {
                        continue;
                    }
                    let id = function_call
                        .get("id")
                        .or_else(|| function_call.get("callId"))
                        .and_then(JsonValue::as_str)
                        .map(str::to_owned)
                        .unwrap_or_else(|| format!("call_{}", self.tool_calls.len() + 1));
                    let arguments = function_call
                        .get("args")
                        .map(|args| serde_json::to_string(args).unwrap_or_default())
                        .unwrap_or_else(|| "{}".to_owned());
                    self.tool_calls.insert(
                        id.clone(),
                        ToolCall {
                            id,
                            name: name.to_owned(),
                            arguments,
                        },
                    );
                }
            }
        }
    }

    fn finalize(&mut self) {
        for call in self.tool_calls.values() {
            self.queue.push_back(LlmEvent::ToolCall(call.clone()));
        }
        self.tool_calls.clear();
        if let Some(usage) = self.usage.take() {
            self.queue.push_back(LlmEvent::Usage(usage));
        }
        self.queue.push_back(LlmEvent::Done {
            reason: self
                .finish_reason
                .clone()
                .unwrap_or_else(|| FinishReason::from_chat(None, self.saw_tool_calls)),
        });
        self.terminated = true;
    }
}

fn parse_usage(value: Option<&JsonValue>) -> Option<Usage> {
    let usage = value?;
    let input_tokens = usage.get("promptTokenCount").and_then(JsonValue::as_u64)? as u32;
    let output_tokens = usage
        .get("candidatesTokenCount")
        .and_then(JsonValue::as_u64)
        .unwrap_or(0) as u32;
    let total_tokens = usage
        .get("totalTokenCount")
        .and_then(JsonValue::as_u64)
        .map(|value| value as u32)
        .unwrap_or_else(|| input_tokens.saturating_add(output_tokens));
    Some(Usage {
        input_tokens,
        output_tokens,
        total_tokens,
        cost: None,
    })
}

fn map_finish_reason(value: &str) -> FinishReason {
    match value {
        "STOP" => FinishReason::Stop,
        "MAX_TOKENS" => FinishReason::Length,
        "SAFETY" | "PROHIBITED_CONTENT" | "SPII" | "BLOCKLIST" => FinishReason::ContentFilter,
        "MALFORMED_FUNCTION_CALL" => FinishReason::ToolCalls,
        other => FinishReason::Other(other.to_ascii_lowercase()),
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use bytes::Bytes;

    use super::*;
    use crate::transport::sse_data_frames;

    #[test]
    fn request_maps_prefixed_google_models_to_native_model_path() {
        let provider = GeminiProvider::new(GeminiConfig {
            base_url: GEMINI_API_BASE.to_owned(),
            api_key: "k".to_owned(),
            provider_name: "google".to_owned(),
            extra_headers: Vec::new(),
            transport: TransportConfig::default(),
        })
        .expect("provider");
        assert_eq!(
            provider.stream_url("google/gemini-2.5-flash"),
            "https://generativelanguage.googleapis.com/v1beta/models/gemini-2.5-flash:streamGenerateContent?alt=sse"
        );
    }

    #[tokio::test]
    async fn normalizer_maps_thought_and_function_call_parts() {
        let stream = futures_util::stream::iter(vec![
            Ok::<Bytes, LlmError>(Bytes::from(
                "data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"thinking\",\"thought\":true}]}}]}\n\n",
            )),
            Ok::<Bytes, LlmError>(Bytes::from(
                "data: {\"candidates\":[{\"content\":{\"parts\":[{\"functionCall\":{\"name\":\"read\",\"args\":{\"path\":\"Cargo.toml\"}}}]},\"finishReason\":\"STOP\"}],\"usageMetadata\":{\"promptTokenCount\":4,\"candidatesTokenCount\":2,\"totalTokenCount\":6}}\n\n",
            )),
            Ok::<Bytes, LlmError>(Bytes::from("data: [DONE]\n\n")),
        ]);
        let bytes: crate::transport::ByteStream = Box::pin(stream);
        let frames = Box::pin(sse_data_frames(bytes, Duration::from_secs(1)));
        let events: Vec<LlmEvent> = normalize_gemini_sse(frames).collect().await;
        assert!(events.iter().any(|event| matches!(
            event,
            LlmEvent::Reasoning { text } if text == "thinking"
        )));
        assert!(
            events
                .iter()
                .any(|event| matches!(event, LlmEvent::ToolCall(call) if call.name == "read"))
        );
        assert!(
            events
                .iter()
                .any(|event| matches!(event, LlmEvent::Usage(usage) if usage.total_tokens == 6))
        );
        assert_eq!(
            events.last(),
            Some(&LlmEvent::Done {
                reason: FinishReason::Stop
            })
        );
    }
}
