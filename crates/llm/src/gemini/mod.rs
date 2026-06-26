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
use crate::media::{encode_media_part, strip_media_stub};
use crate::provider::{LlmStream, Provider};
use crate::request::{LlmRequest, MessageContentPart, Role, ToolChoice};
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
    let mut normalized = request.clone();
    normalized.normalize_known_params();
    let request = &normalized;
    let mut contents = Vec::new();
    let mut system_parts = Vec::new();
    for message in &request.messages {
        match message.role {
            Role::System => {
                let mut text = message.content.clone();
                if !message.parts.is_empty() {
                    if !text.is_empty() {
                        text.push('\n');
                    }
                    text.push_str(
                        &message
                            .parts
                            .iter()
                            .map(strip_media_stub)
                            .collect::<Vec<_>>()
                            .join("\n"),
                    );
                }
                if !text.is_empty() {
                    system_parts.push(serde_json::json!({ "text": text }));
                }
            }
            Role::User => {
                let mut parts = Vec::new();
                if !message.content.is_empty() {
                    parts.push(serde_json::json!({ "text": message.content }));
                }
                let (media_parts, strip_stubs) = gemini_media_parts(&message.parts);
                parts.extend(media_parts);
                if !strip_stubs.is_empty() {
                    parts.push(serde_json::json!({ "text": strip_stubs.join("\n") }));
                }
                contents.push(serde_json::json!({
                    "role": "user",
                    "parts": parts,
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
                let mut strip_stubs = Vec::new();
                for part in &message.parts {
                    match part {
                        MessageContentPart::Reasoning {
                            provider,
                            text,
                            signature,
                        } if provider.eq_ignore_ascii_case("gemini")
                            || provider.eq_ignore_ascii_case("google") =>
                        {
                            if text.is_empty() {
                                continue;
                            }
                            let mut thought_part = serde_json::json!({
                                "text": text,
                                "thought": true,
                            });
                            if let Some(signature) = signature {
                                thought_part["thoughtSignature"] = serde_json::json!(signature);
                            }
                            parts.push(thought_part);
                        }
                        _ => strip_stubs.push(strip_media_stub(part)),
                    }
                }
                if !strip_stubs.is_empty() {
                    parts.push(serde_json::json!({
                        "text": strip_stubs.join("\n"),
                    }));
                }
                contents.push(serde_json::json!({
                    "role": "model",
                    "parts": parts,
                }));
            }
            Role::Tool => {
                let (media_parts, strip_stubs) = gemini_media_parts(&message.parts);
                let mut parts = vec![serde_json::json!({
                    "functionResponse": {
                        "name": message
                            .tool_call_id
                            .clone()
                            .unwrap_or_else(|| "tool_result".to_owned()),
                        "response": { "content": message.content },
                    }
                })];
                parts.extend(media_parts);
                if !strip_stubs.is_empty() {
                    parts.push(serde_json::json!({ "text": strip_stubs.join("\n") }));
                }
                contents.push(serde_json::json!({
                    "role": "user",
                    "parts": parts,
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
    let mut generation_config = serde_json::Map::new();
    let temperature = request
        .temperature
        .map(clamp_gemini_temperature)
        .unwrap_or(1.0);
    generation_config.insert("temperature".to_owned(), serde_json::json!(temperature));
    generation_config.insert("topP".to_owned(), serde_json::json!(0.95));
    if let Some(max_tokens) = request.max_tokens {
        generation_config.insert("maxOutputTokens".to_owned(), serde_json::json!(max_tokens));
    }

    if let Some(tool_config) = gemini_tool_config(request.tool_choice.as_ref()) {
        body["toolConfig"] = tool_config;
    }

    for (name, param) in &request.extra_params {
        match name.as_str() {
            "temperature" => {
                if let Some(value) = param.as_f64() {
                    generation_config.insert(
                        "temperature".to_owned(),
                        serde_json::json!(clamp_gemini_temperature(value as f32)),
                    );
                }
            }
            "top_p" | "topP" => {
                if let Some(value) = param.as_f64() {
                    generation_config.insert(
                        "topP".to_owned(),
                        serde_json::json!(clamp_unit_interval(value)),
                    );
                }
            }
            "top_k" | "topK" => {
                if let Some(value) = parse_positive_u32(param) {
                    generation_config.insert("topK".to_owned(), serde_json::json!(value));
                }
            }
            "candidate_count" | "candidateCount" => {
                if let Some(value) = parse_positive_u32(param) {
                    generation_config.insert(
                        "candidateCount".to_owned(),
                        serde_json::json!(value.clamp(1, 8)),
                    );
                }
            }
            "stop" | "stop_sequences" | "stopSequences" => {
                if let Some(sequences) = stop_sequences_from_value(param) {
                    generation_config.insert("stopSequences".to_owned(), sequences);
                }
            }
            "reasoning_effort" | "reasoningEffort" | "thinking_budget" | "thinkingBudget" => {
                if let Some(budget) = reasoning_budget_from_value(param) {
                    apply_gemini_thinking_budget(&mut generation_config, budget);
                }
            }
            "max_tokens" => {
                if let Some(value) = parse_positive_u32(param) {
                    generation_config
                        .insert("maxOutputTokens".to_owned(), serde_json::json!(value));
                }
            }
            _ => {
                if !is_reserved_gemini_param(name) {
                    // Scrub unknown provider params to prevent unsupported-parameter leakage.
                }
            }
        }
    }
    body["generationConfig"] = JsonValue::Object(generation_config);
    serde_json::to_string(&body)
        .map_err(|err| LlmError::transport(format!("failed to encode request: {err}")))
}

fn gemini_media_parts(parts: &[MessageContentPart]) -> (Vec<JsonValue>, Vec<String>) {
    let mut out = Vec::new();
    let mut strip_stubs = Vec::new();
    for part in parts {
        if matches!(part, MessageContentPart::Reasoning { .. }) {
            continue;
        }
        let Ok(encoded) = encode_media_part(part) else {
            strip_stubs.push(strip_media_stub(part));
            continue;
        };
        out.push(serde_json::json!({
            "inlineData": {
                "mimeType": encoded.media_type,
                "data": encoded.base64,
            }
        }));
    }
    (out, strip_stubs)
}

fn is_reserved_gemini_param(name: &str) -> bool {
    matches!(
        name,
        "contents"
            | "systemInstruction"
            | "tools"
            | "generationConfig"
            | "toolConfig"
            | "temperature"
            | "top_p"
            | "topP"
            | "top_k"
            | "topK"
            | "candidate_count"
            | "candidateCount"
            | "stop"
            | "stop_sequences"
            | "stopSequences"
            | "reasoning_effort"
            | "reasoningEffort"
            | "thinking_budget"
            | "thinkingBudget"
            | "tool_choice"
            | "toolChoice"
            | "max_tokens"
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

fn gemini_tool_config(tool_choice: Option<&ToolChoice>) -> Option<JsonValue> {
    match tool_choice {
        Some(ToolChoice::Auto) => Some(serde_json::json!({
            "functionCallingConfig": { "mode": "AUTO" }
        })),
        Some(ToolChoice::None) => Some(serde_json::json!({
            "functionCallingConfig": { "mode": "NONE" }
        })),
        Some(ToolChoice::Required) => Some(serde_json::json!({
            "functionCallingConfig": { "mode": "ANY" }
        })),
        Some(ToolChoice::Tool { name }) if !name.trim().is_empty() => Some(serde_json::json!({
            "functionCallingConfig": {
                "mode": "ANY",
                "allowedFunctionNames": [name],
            }
        })),
        Some(ToolChoice::Tool { .. }) => Some(serde_json::json!({
            "functionCallingConfig": { "mode": "ANY" }
        })),
        None => None,
    }
}

fn clamp_gemini_temperature(value: f32) -> f32 {
    value.clamp(0.0, 2.0)
}

fn clamp_unit_interval(value: f64) -> f64 {
    value.clamp(0.0, 1.0)
}

fn parse_positive_u32(value: &JsonValue) -> Option<u32> {
    value
        .as_u64()
        .and_then(|value| u32::try_from(value).ok())
        .or_else(|| value.as_str().and_then(|value| value.parse::<u32>().ok()))
        .filter(|value| *value > 0)
}

fn stop_sequences_from_value(value: &JsonValue) -> Option<JsonValue> {
    match value {
        JsonValue::String(sequence) => Some(serde_json::json!([sequence])),
        JsonValue::Array(items) => {
            let sequences = items
                .iter()
                .filter_map(JsonValue::as_str)
                .map(ToOwned::to_owned)
                .collect::<Vec<_>>();
            (!sequences.is_empty()).then(|| serde_json::json!(sequences))
        }
        _ => None,
    }
}

fn reasoning_budget_from_value(value: &JsonValue) -> Option<u32> {
    match value {
        JsonValue::String(value) => match value.trim().to_ascii_lowercase().as_str() {
            "minimal" => Some(1_024),
            "low" => Some(2_048),
            "medium" => Some(4_096),
            "high" => Some(8_192),
            _ => None,
        },
        JsonValue::Number(_) => parse_positive_u32(value).map(|budget| budget.clamp(1_024, 8_192)),
        _ => None,
    }
}

fn apply_gemini_thinking_budget(config: &mut serde_json::Map<String, JsonValue>, budget: u32) {
    let mut thinking = config
        .get("thinkingConfig")
        .and_then(JsonValue::as_object)
        .cloned()
        .unwrap_or_default();
    thinking.insert("thinkingBudget".to_owned(), serde_json::json!(budget));
    config.insert("thinkingConfig".to_owned(), JsonValue::Object(thinking));
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
                            let metadata = part
                                .get("thoughtSignature")
                                .and_then(JsonValue::as_str)
                                .map(|signature| {
                                    serde_json::json!({
                                        "provider": "gemini",
                                        "signature": signature,
                                    })
                                });
                            self.queue.push_back(LlmEvent::Reasoning {
                                text: text.to_owned(),
                                metadata,
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
        cache_creation_input_tokens: None,
        cache_read_input_tokens: None,
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
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    use bytes::Bytes;

    use super::*;
    use crate::request::{Message, MessageContentPart};
    use crate::transport::sse_data_frames;

    fn temp_file(name: &str, bytes: &[u8]) -> PathBuf {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "cortex-gemini-wire-{name}-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        fs::write(&path, bytes).expect("write temp attachment");
        path
    }

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

    #[test]
    fn user_image_parts_serialize_to_inline_data() {
        let image = temp_file(
            "img.png",
            &[
                0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1a, b'\n', 0, 0, 0, 0x0d, b'I', b'H', b'D',
                b'R', 0, 0, 0, 0x01, 0, 0, 0, 0x01, 0x08, 0x02, 0, 0, 0, 0x90, 0x77, 0x53, 0xde, 0,
                0, 0, 0x0a, b'I', b'D', b'A', b'T', 0x08, 0xd7, 0x63, 0xf8, 0x0f, 0x00, 0x01, 0x05,
                0x01, 0x02, 0x60, 0x5d, 0xe9, 0x2f, 0, 0, 0, 0, b'I', b'E', b'N', b'D', 0xae, 0x42,
                0x60, 0x82,
            ],
        );
        let request = LlmRequest::new("google/gemini-2.5-flash").message(
            Message::user("inspect image").with_parts(vec![MessageContentPart::Image {
                media_type: "image/png".to_owned(),
                path: image.to_string_lossy().into_owned(),
            }]),
        );
        let encoded = build_gemini_request(&request).expect("request");
        let json: JsonValue = serde_json::from_str(&encoded).expect("json");
        assert!(json["contents"][0]["parts"][1]["inlineData"].is_object());
        assert_eq!(
            json["contents"][0]["parts"][1]["inlineData"]["mimeType"],
            "image/png"
        );
    }

    #[test]
    fn tool_results_include_media_inline_data_parts() {
        let image = temp_file(
            "tool.png",
            &[
                0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1a, b'\n', 0, 0, 0, 0x0d, b'I', b'H', b'D',
                b'R', 0, 0, 0, 0x01, 0, 0, 0, 0x01, 0x08, 0x02, 0, 0, 0, 0x90, 0x77, 0x53, 0xde, 0,
                0, 0, 0x0a, b'I', b'D', b'A', b'T', 0x08, 0xd7, 0x63, 0xf8, 0x0f, 0x00, 0x01, 0x05,
                0x01, 0x02, 0x60, 0x5d, 0xe9, 0x2f, 0, 0, 0, 0, b'I', b'E', b'N', b'D', 0xae, 0x42,
                0x60, 0x82,
            ],
        );
        let request = LlmRequest::new("google/gemini-2.5-flash").message(
            Message::tool_result("call_1", "tool output").with_parts(vec![
                MessageContentPart::Image {
                    media_type: "image/png".to_owned(),
                    path: image.to_string_lossy().into_owned(),
                },
            ]),
        );
        let encoded = build_gemini_request(&request).expect("request");
        let json: JsonValue = serde_json::from_str(&encoded).expect("json");
        assert!(json["contents"][0]["parts"][1]["inlineData"].is_object());
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
            LlmEvent::Reasoning { text, .. } if text == "thinking"
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
