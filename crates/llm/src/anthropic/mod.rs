//! Native Anthropic Messages-protocol adapter.
//!
//! This adapter speaks Anthropic's `/messages` SSE protocol directly, including tool-use
//! (`tool_use`) and tool-result (`tool_result`) mapping.

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
use crate::media::{encode_media_part, is_image_part, strip_media_stub};
use crate::provider::{LlmStream, Provider};
use crate::request::{LlmRequest, MessageContentPart, Role, ToolChoice};
use crate::transport::{HttpTransport, TransportConfig, sse_data_frames};

/// Default Anthropic API base (without `/messages`).
pub const ANTHROPIC_API_BASE: &str = "https://api.anthropic.com/v1";
const ANTHROPIC_VERSION: &str = "2023-06-01";
const OUTBOUND_OPERATION: &str = "LLM streaming request";
const PROMPT_CACHE_BREAKPOINT_CAP: usize = 4;
const PROMPT_CACHE_PARAM: &str = "prompt_cache";
const PROMPT_CACHE_TTL_SECONDS_PARAM: &str = "prompt_cache_ttl_seconds";

/// Configuration for [`AnthropicMessages`].
#[derive(Clone)]
pub struct AnthropicConfig {
    pub base_url: String,
    pub api_key: String,
    pub provider_name: String,
    pub extra_headers: Vec<(String, String)>,
    pub transport: TransportConfig,
}

impl fmt::Debug for AnthropicConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AnthropicConfig")
            .field("base_url", &self.base_url)
            .field("api_key", &PLACEHOLDER)
            .field("provider_name", &self.provider_name)
            .field("extra_headers", &self.extra_headers)
            .field("transport", &self.transport)
            .finish()
    }
}

/// Native Anthropic Messages provider.
#[derive(Clone)]
pub struct AnthropicMessages {
    transport: HttpTransport,
    base_url: String,
    api_key: String,
    provider_name: String,
    extra_headers: Vec<(String, String)>,
}

impl AnthropicMessages {
    pub fn new(config: AnthropicConfig) -> Result<Self, LlmError> {
        let transport = HttpTransport::new(config.transport)?;
        Ok(Self {
            transport,
            base_url: config.base_url,
            api_key: config.api_key,
            provider_name: config.provider_name,
            extra_headers: config.extra_headers,
        })
    }

    fn messages_url(&self) -> String {
        format!("{}/messages", self.base_url.trim_end_matches('/'))
    }
}

impl fmt::Debug for AnthropicMessages {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AnthropicMessages")
            .field("transport", &self.transport)
            .field("base_url", &self.base_url)
            .field("api_key", &PLACEHOLDER)
            .field("provider_name", &self.provider_name)
            .field("extra_headers", &self.extra_headers)
            .finish()
    }
}

impl Provider for AnthropicMessages {
    fn stream<'a>(&'a self, request: LlmRequest) -> BoxFuture<'a, Result<LlmStream, LlmError>> {
        Box::pin(async move {
            guard_outbound(OUTBOUND_OPERATION).map_err(LlmError::from_diagnostic)?;

            if self.api_key.is_empty() {
                return Err(LlmError::no_credential(&self.provider_name));
            }

            let body = build_messages_request(&request)?;
            let mut headers = vec![
                ("x-api-key".to_owned(), self.api_key.clone()),
                ("anthropic-version".to_owned(), ANTHROPIC_VERSION.to_owned()),
            ];
            headers.extend(self.extra_headers.clone());
            headers.extend(request.extra_headers.iter().cloned());

            let bytes = self
                .transport
                .open_post_sse_with_headers(&self.messages_url(), &headers, body)
                .await?;
            let frames = Box::pin(sse_data_frames(bytes, self.transport.config().idle_timeout));
            Ok(Box::pin(normalize_messages_sse(frames)) as LlmStream)
        })
    }
}

fn build_messages_request(request: &LlmRequest) -> Result<String, LlmError> {
    let mut normalized = request.clone();
    normalized.normalize_known_params();
    let request = &normalized;
    let model = normalize_model_id(&request.model);
    let mut system_parts = Vec::new();
    let mut messages = Vec::new();
    let mut user_text_positions = Vec::new();
    let mut tool_result_positions = Vec::new();

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
                    system_parts.push(serde_json::json!({
                        "type": "text",
                        "text": text,
                    }));
                }
            }
            Role::User => {
                let mut content = Vec::new();
                if !message.content.is_empty() {
                    content.push(serde_json::json!({
                        "type": "text",
                        "text": message.content,
                    }));
                    user_text_positions.push((messages.len(), content.len() - 1));
                }
                let (media_blocks, strip_stubs) = anthropic_media_blocks(&message.parts);
                content.extend(media_blocks);
                if !strip_stubs.is_empty() {
                    content.push(serde_json::json!({
                        "type": "text",
                        "text": strip_stubs.join("\n"),
                    }));
                    user_text_positions.push((messages.len(), content.len() - 1));
                }
                messages.push(serde_json::json!({
                    "role": "user",
                    "content": content,
                }));
            }
            Role::Assistant => {
                let mut content = Vec::new();
                if !message.content.is_empty() {
                    content.push(serde_json::json!({"type": "text", "text": message.content}));
                }
                for call in &message.tool_calls {
                    content.push(serde_json::json!({
                        "type": "tool_use",
                        "id": call.id,
                        "name": call.name,
                        "input": parse_json_arguments(&call.arguments),
                    }));
                }
                let mut strip_stubs = Vec::new();
                for part in &message.parts {
                    match part {
                        MessageContentPart::Reasoning {
                            provider,
                            text,
                            signature,
                        } if provider.eq_ignore_ascii_case("anthropic") => {
                            if text.is_empty() {
                                continue;
                            }
                            let mut block = serde_json::json!({
                                "type": "thinking",
                                "thinking": text,
                            });
                            if let Some(signature) = signature {
                                block["signature"] = serde_json::json!(signature);
                            }
                            content.push(block);
                        }
                        _ => strip_stubs.push(strip_media_stub(part)),
                    }
                }
                if !strip_stubs.is_empty() {
                    content.push(serde_json::json!({
                        "type": "text",
                        "text": strip_stubs.join("\n"),
                    }));
                }
                messages.push(serde_json::json!({
                    "role": "assistant",
                    "content": content,
                }));
            }
            Role::Tool => {
                let (media_blocks, strip_stubs) = anthropic_media_blocks(&message.parts);
                let mut tool_content = Vec::new();
                if !message.content.is_empty() {
                    tool_content.push(serde_json::json!({
                        "type": "text",
                        "text": message.content,
                    }));
                }
                tool_content.extend(media_blocks);
                if !strip_stubs.is_empty() {
                    tool_content.push(serde_json::json!({
                        "type": "text",
                        "text": strip_stubs.join("\n"),
                    }));
                }
                messages.push(serde_json::json!({
                    "role": "user",
                    "content": [{
                        "type": "tool_result",
                        "tool_use_id": message.tool_call_id.clone().unwrap_or_else(|| "tool_call".to_owned()),
                        "content": if tool_content.is_empty() {
                            JsonValue::String(message.content.clone())
                        } else {
                            JsonValue::Array(tool_content)
                        },
                    }],
                }));
                tool_result_positions.push((messages.len() - 1, 0usize));
            }
        }
    }

    if prompt_cache_enabled(request) {
        let cache_control = prompt_cache_control(request);
        let mut remaining = PROMPT_CACHE_BREAKPOINT_CAP;
        if let Some((message_index, block_index)) = tool_result_positions.last().copied() {
            if apply_cache_control_to_message_block(
                &mut messages,
                message_index,
                block_index,
                &cache_control,
            ) {
                remaining -= 1;
            }
        }
        if remaining > 0
            && let Some(system_index) = system_parts.len().checked_sub(1)
            && apply_cache_control_to_system_block(&mut system_parts, system_index, &cache_control)
        {
            remaining -= 1;
        }
        if remaining > 0
            && let Some((message_index, block_index)) = user_text_positions.last().copied()
        {
            let _ = apply_cache_control_to_message_block(
                &mut messages,
                message_index,
                block_index,
                &cache_control,
            );
        }
    }

    let mut body = serde_json::json!({
        "model": model,
        "messages": messages,
        "stream": true,
        "max_tokens": request.max_tokens.unwrap_or(4096),
    });
    if !system_parts.is_empty() {
        body["system"] = serde_json::Value::Array(system_parts);
    }
    let mut suppress_tools = false;
    if let Some((tool_choice, suppress)) = anthropic_tool_choice(request.tool_choice.as_ref()) {
        suppress_tools = suppress;
        body["tool_choice"] = tool_choice;
    } else if matches!(request.tool_choice, Some(ToolChoice::None)) {
        suppress_tools = true;
    }
    if !request.tools.is_empty() && !suppress_tools {
        body["tools"] = serde_json::Value::Array(
            request
                .tools
                .iter()
                .map(|tool| {
                    serde_json::json!({
                        "name": tool.name,
                        "description": tool.description,
                        "input_schema": tool.parameters,
                    })
                })
                .collect(),
        );
    }

    let mut temperature = request.temperature.map(clamp_anthropic_temperature);
    let mut top_p = None;
    if let Some(map) = body.as_object_mut() {
        for (name, param) in &request.extra_params {
            match name.as_str() {
                "temperature" => {
                    if let Some(value) = param.as_f64() {
                        temperature = Some(clamp_anthropic_temperature(value as f32));
                    }
                }
                "top_p" | "topP" => {
                    if let Some(value) = param.as_f64() {
                        top_p = Some(clamp_unit_interval(value));
                    }
                }
                "top_k" | "topK" => {
                    if let Some(value) = parse_positive_u32(param) {
                        map.insert("top_k".to_owned(), serde_json::json!(value));
                    }
                }
                "stop" | "stop_sequences" | "stopSequences" => {
                    if let Some(sequences) = stop_sequences_from_value(param) {
                        map.insert("stop_sequences".to_owned(), sequences);
                    }
                }
                "metadata" => {
                    if param.is_object() {
                        map.insert("metadata".to_owned(), param.clone());
                    }
                }
                "thinking" => {
                    if param.is_object() {
                        map.insert("thinking".to_owned(), param.clone());
                    }
                }
                "reasoning_effort" | "reasoningEffort" | "thinking_budget" | "thinkingBudget" => {
                    if let Some(budget) = reasoning_budget_from_value(param) {
                        apply_anthropic_thinking_budget(map, budget);
                    }
                }
                "max_tokens" => {
                    if let Some(value) = parse_positive_u32(param) {
                        map.insert("max_tokens".to_owned(), serde_json::json!(value));
                    }
                }
                _ => {
                    if !is_reserved_messages_param(name) {
                        // Scrub unknown provider params to prevent unsupported-parameter leakage.
                    }
                }
            }
        }
        map.insert(
            "temperature".to_owned(),
            serde_json::json!(temperature.unwrap_or(1.0)),
        );
        map.insert(
            "top_p".to_owned(),
            serde_json::json!(top_p.unwrap_or(0.999)),
        );
    }
    serde_json::to_string(&body)
        .map_err(|err| LlmError::transport(format!("failed to encode request: {err}")))
}

fn anthropic_media_blocks(parts: &[MessageContentPart]) -> (Vec<JsonValue>, Vec<String>) {
    let mut blocks = Vec::new();
    let mut strip_stubs = Vec::new();
    for part in parts {
        if matches!(part, MessageContentPart::Reasoning { .. }) {
            continue;
        }
        let Ok(encoded) = encode_media_part(part) else {
            strip_stubs.push(strip_media_stub(part));
            continue;
        };
        if is_image_part(part) {
            blocks.push(serde_json::json!({
                "type": "image",
                "source": {
                    "type": "base64",
                    "media_type": encoded.media_type,
                    "data": encoded.base64,
                }
            }));
        } else {
            blocks.push(serde_json::json!({
                "type": "document",
                "source": {
                    "type": "base64",
                    "media_type": encoded.media_type,
                    "data": encoded.base64,
                }
            }));
        }
    }
    (blocks, strip_stubs)
}

fn prompt_cache_enabled(request: &LlmRequest) -> bool {
    request
        .extra_params
        .get(PROMPT_CACHE_PARAM)
        .and_then(JsonValue::as_str)
        .map(|mode| !mode.eq_ignore_ascii_case("off"))
        .or_else(|| {
            request
                .extra_params
                .get(PROMPT_CACHE_PARAM)
                .and_then(JsonValue::as_bool)
        })
        .unwrap_or(true)
}

fn prompt_cache_control(request: &LlmRequest) -> JsonValue {
    let ttl = request
        .extra_params
        .get(PROMPT_CACHE_TTL_SECONDS_PARAM)
        .and_then(JsonValue::as_u64)
        .filter(|seconds| *seconds >= 3_600)
        .map(|_| "1h");
    match ttl {
        Some(ttl) => serde_json::json!({
            "type": "ephemeral",
            "ttl": ttl,
        }),
        None => serde_json::json!({
            "type": "ephemeral",
        }),
    }
}

fn apply_cache_control_to_system_block(
    system: &mut [JsonValue],
    index: usize,
    cache_control: &JsonValue,
) -> bool {
    let Some(block) = system.get_mut(index).and_then(JsonValue::as_object_mut) else {
        return false;
    };
    block.insert("cache_control".to_owned(), cache_control.clone());
    true
}

fn apply_cache_control_to_message_block(
    messages: &mut [JsonValue],
    message_index: usize,
    block_index: usize,
    cache_control: &JsonValue,
) -> bool {
    let Some(content) = messages
        .get_mut(message_index)
        .and_then(JsonValue::as_object_mut)
        .and_then(|message| message.get_mut("content"))
        .and_then(JsonValue::as_array_mut)
    else {
        return false;
    };
    let Some(block) = content
        .get_mut(block_index)
        .and_then(JsonValue::as_object_mut)
    else {
        return false;
    };
    block.insert("cache_control".to_owned(), cache_control.clone());
    true
}

fn is_reserved_messages_param(name: &str) -> bool {
    matches!(
        name,
        "model"
            | "messages"
            | "stream"
            | "system"
            | "tools"
            | "max_tokens"
            | "temperature"
            | "top_p"
            | "topP"
            | "top_k"
            | "topK"
            | "stop"
            | "stop_sequences"
            | "stopSequences"
            | "metadata"
            | "thinking"
            | "reasoning_effort"
            | "reasoningEffort"
            | "thinking_budget"
            | "thinkingBudget"
            | "tool_choice"
            | "toolChoice"
            | PROMPT_CACHE_PARAM
            | PROMPT_CACHE_TTL_SECONDS_PARAM
    )
}

fn normalize_model_id(model_id: &str) -> String {
    let Some((prefix, rest)) = model_id.split_once('/') else {
        return model_id.to_owned();
    };
    if prefix.eq_ignore_ascii_case("anthropic") {
        rest.to_owned()
    } else {
        model_id.to_owned()
    }
}

fn parse_json_arguments(arguments: &str) -> JsonValue {
    serde_json::from_str(arguments).unwrap_or_else(|_| serde_json::json!({ "raw": arguments }))
}

fn anthropic_tool_choice(tool_choice: Option<&ToolChoice>) -> Option<(JsonValue, bool)> {
    match tool_choice {
        Some(ToolChoice::Auto) => Some((serde_json::json!({ "type": "auto" }), false)),
        Some(ToolChoice::Required) => Some((serde_json::json!({ "type": "any" }), false)),
        Some(ToolChoice::Tool { name }) if !name.trim().is_empty() => Some((
            serde_json::json!({
                "type": "tool",
                "name": name,
            }),
            false,
        )),
        Some(ToolChoice::Tool { .. }) => Some((serde_json::json!({ "type": "any" }), false)),
        Some(ToolChoice::None) => None,
        None => None,
    }
}

fn clamp_anthropic_temperature(value: f32) -> f32 {
    value.clamp(0.0, 1.0)
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

fn apply_anthropic_thinking_budget(map: &mut serde_json::Map<String, JsonValue>, budget: u32) {
    let mut thinking = map
        .get("thinking")
        .and_then(JsonValue::as_object)
        .cloned()
        .unwrap_or_default();
    thinking.insert("type".to_owned(), serde_json::json!("enabled"));
    thinking.insert("budget_tokens".to_owned(), serde_json::json!(budget));
    map.insert("thinking".to_owned(), JsonValue::Object(thinking));
}

type FrameStream = std::pin::Pin<Box<dyn Stream<Item = Result<String, LlmError>> + Send>>;

fn normalize_messages_sse(frames: FrameStream) -> impl Stream<Item = LlmEvent> + Send {
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
    tools: BTreeMap<usize, ToolAccumulator>,
    usage: Usage,
    saw_usage: bool,
    saw_tool_calls: bool,
    stop_reason: Option<String>,
    terminated: bool,
}

impl NormalizeState {
    fn new(frames: FrameStream) -> Self {
        Self {
            frames,
            queue: VecDeque::new(),
            tools: BTreeMap::new(),
            usage: Usage {
                input_tokens: 0,
                output_tokens: 0,
                total_tokens: 0,
                cache_creation_input_tokens: None,
                cache_read_input_tokens: None,
                cost: None,
            },
            saw_usage: false,
            saw_tool_calls: false,
            stop_reason: None,
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
                self.terminated = true;
                self.queue.push_back(LlmEvent::Done {
                    reason: FinishReason::Error,
                });
            }
            Ok(data) if data.trim() == "[DONE]" => self.finalize(),
            Ok(data) => self.consume(&data),
        }
    }

    fn consume(&mut self, data: &str) {
        let Ok(value) = serde_json::from_str::<JsonValue>(data) else {
            return;
        };
        let event_type = value
            .get("type")
            .and_then(JsonValue::as_str)
            .unwrap_or_default();
        match event_type {
            "message_start" => {
                if let Some(usage) = value
                    .get("message")
                    .and_then(|message| message.get("usage"))
                {
                    if let Some(input) = usage.get("input_tokens").and_then(JsonValue::as_u64) {
                        self.usage.input_tokens = input as u32;
                        self.saw_usage = true;
                    }
                    update_cache_usage_fields(&mut self.usage, usage);
                }
            }
            "content_block_start" => {
                let index = value.get("index").and_then(JsonValue::as_u64).unwrap_or(0) as usize;
                let Some(block) = value.get("content_block") else {
                    return;
                };
                match block.get("type").and_then(JsonValue::as_str) {
                    Some("text") => {
                        if let Some(text) = block.get("text").and_then(JsonValue::as_str)
                            && !text.is_empty()
                        {
                            self.queue.push_back(LlmEvent::TextDelta {
                                text: text.to_owned(),
                            });
                        }
                    }
                    Some("tool_use") => {
                        self.saw_tool_calls = true;
                        let id = block
                            .get("id")
                            .and_then(JsonValue::as_str)
                            .unwrap_or_default()
                            .to_owned();
                        let name = block
                            .get("name")
                            .and_then(JsonValue::as_str)
                            .unwrap_or_default()
                            .to_owned();
                        let mut arguments = String::new();
                        if let Some(input) = block.get("input") {
                            arguments = serde_json::to_string(input).unwrap_or_default();
                        }
                        self.tools.insert(
                            index,
                            ToolAccumulator {
                                id,
                                name,
                                arguments,
                            },
                        );
                    }
                    _ => {}
                }
            }
            "content_block_delta" => {
                let index = value.get("index").and_then(JsonValue::as_u64).unwrap_or(0) as usize;
                let Some(delta) = value.get("delta") else {
                    return;
                };
                match delta.get("type").and_then(JsonValue::as_str) {
                    Some("text_delta") => {
                        if let Some(text) = delta.get("text").and_then(JsonValue::as_str)
                            && !text.is_empty()
                        {
                            self.queue.push_back(LlmEvent::TextDelta {
                                text: text.to_owned(),
                            });
                        }
                    }
                    Some("thinking_delta") => {
                        if let Some(text) = delta.get("thinking").and_then(JsonValue::as_str)
                            && !text.is_empty()
                        {
                            self.queue.push_back(LlmEvent::Reasoning {
                                text: text.to_owned(),
                                metadata: None,
                            });
                        }
                    }
                    Some("signature_delta") => {
                        if let Some(signature) = delta.get("signature").and_then(JsonValue::as_str)
                            && !signature.is_empty()
                        {
                            self.queue.push_back(LlmEvent::Reasoning {
                                text: String::new(),
                                metadata: Some(serde_json::json!({
                                    "provider": "anthropic",
                                    "signature": signature,
                                })),
                            });
                        }
                    }
                    Some("input_json_delta") => {
                        self.saw_tool_calls = true;
                        if let Some(partial) = delta.get("partial_json").and_then(JsonValue::as_str)
                        {
                            self.tools
                                .entry(index)
                                .or_default()
                                .arguments
                                .push_str(partial);
                        }
                    }
                    _ => {}
                }
            }
            "message_delta" => {
                if let Some(stop_reason) = value
                    .get("delta")
                    .and_then(|delta| delta.get("stop_reason"))
                    .and_then(JsonValue::as_str)
                {
                    self.stop_reason = Some(stop_reason.to_owned());
                }
                if let Some(usage) = value.get("usage") {
                    if let Some(output) = usage.get("output_tokens").and_then(JsonValue::as_u64) {
                        self.usage.output_tokens = output as u32;
                        self.saw_usage = true;
                    }
                    update_cache_usage_fields(&mut self.usage, usage);
                }
            }
            "error" => {
                let message = value
                    .get("error")
                    .and_then(|error| error.get("message"))
                    .and_then(JsonValue::as_str)
                    .unwrap_or("unspecified upstream error");
                self.queue.push_back(LlmEvent::Error {
                    message: redaction::redact(message),
                    kind: "upstream".to_owned(),
                });
                self.terminated = true;
                self.queue.push_back(LlmEvent::Done {
                    reason: FinishReason::Error,
                });
            }
            _ => {}
        }
    }

    fn finalize(&mut self) {
        for tool in self.tools.values() {
            self.queue.push_back(LlmEvent::ToolCall(ToolCall {
                id: tool.id.clone(),
                name: tool.name.clone(),
                arguments: tool.arguments.clone(),
            }));
        }
        self.tools.clear();
        if self.saw_usage {
            self.usage.total_tokens = self
                .usage
                .input_tokens
                .saturating_add(self.usage.output_tokens);
            self.queue.push_back(LlmEvent::Usage(self.usage.clone()));
        }
        self.queue.push_back(LlmEvent::Done {
            reason: finish_reason(self.stop_reason.as_deref(), self.saw_tool_calls),
        });
        self.terminated = true;
    }
}

fn update_cache_usage_fields(usage: &mut Usage, value: &JsonValue) {
    if let Some(tokens) = value
        .get("cache_creation_input_tokens")
        .and_then(JsonValue::as_u64)
        .and_then(|tokens| u32::try_from(tokens).ok())
    {
        usage.cache_creation_input_tokens = Some(tokens);
    }
    if let Some(tokens) = value
        .get("cache_read_input_tokens")
        .and_then(JsonValue::as_u64)
        .and_then(|tokens| u32::try_from(tokens).ok())
    {
        usage.cache_read_input_tokens = Some(tokens);
    }
}

fn finish_reason(stop_reason: Option<&str>, saw_tool_calls: bool) -> FinishReason {
    match stop_reason {
        Some("end_turn") | Some("stop_sequence") => FinishReason::Stop,
        Some("max_tokens") => FinishReason::Length,
        Some("tool_use") => FinishReason::ToolCalls,
        Some("content_filter") => FinishReason::ContentFilter,
        Some(other) => FinishReason::Other(other.to_owned()),
        None if saw_tool_calls => FinishReason::ToolCalls,
        None => FinishReason::Stop,
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
            "cortex-anthropic-wire-{name}-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        fs::write(&path, bytes).expect("write temp attachment");
        path
    }

    #[test]
    fn request_maps_tool_results_to_tool_result_blocks() {
        let request = LlmRequest::new("anthropic/claude-sonnet-4")
            .message(Message::tool_result("call_1", "ok"));
        let encoded = build_messages_request(&request).expect("request");
        let json: JsonValue = serde_json::from_str(&encoded).expect("json");
        assert_eq!(json["model"], "claude-sonnet-4");
        assert!(
            json["messages"]
                .as_array()
                .expect("messages array")
                .iter()
                .any(|entry| {
                    entry["content"][0]["type"] == "tool_result"
                        && entry["content"][0]["tool_use_id"] == "call_1"
                })
        );
    }

    #[test]
    fn user_image_parts_serialize_to_image_blocks() {
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
        let request = LlmRequest::new("anthropic/claude-sonnet-4").message(
            Message::user("inspect image").with_parts(vec![MessageContentPart::Image {
                media_type: "image/png".to_owned(),
                path: image.to_string_lossy().into_owned(),
            }]),
        );
        let encoded = build_messages_request(&request).expect("request");
        let json: JsonValue = serde_json::from_str(&encoded).expect("json");
        assert_eq!(json["messages"][0]["content"][1]["type"], "image");
        assert_eq!(
            json["messages"][0]["content"][1]["source"]["type"],
            "base64"
        );
    }

    #[test]
    fn tool_results_can_include_media_arrays() {
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
        let request = LlmRequest::new("anthropic/claude-sonnet-4").message(
            Message::tool_result("call_1", "tool output").with_parts(vec![
                MessageContentPart::Image {
                    media_type: "image/png".to_owned(),
                    path: image.to_string_lossy().into_owned(),
                },
            ]),
        );
        let encoded = build_messages_request(&request).expect("request");
        let json: JsonValue = serde_json::from_str(&encoded).expect("json");
        assert_eq!(
            json["messages"][0]["content"][0]["content"][1]["type"],
            "image"
        );
    }

    #[test]
    fn prompt_cache_breakpoints_mark_last_system_tool_and_user_text_blocks() {
        let request = LlmRequest::new("anthropic/claude-sonnet-4")
            .message(Message::system("system instructions"))
            .message(Message::user("first user message"))
            .message({
                let mut assistant = Message::assistant("");
                assistant.tool_calls.push(ToolCall {
                    id: "call_1".to_owned(),
                    name: "read".to_owned(),
                    arguments: "{\"path\":\"Cargo.toml\"}".to_owned(),
                });
                assistant
            })
            .message(Message::tool_result("call_1", "file contents"))
            .message(Message::user("second user message"))
            .extra_param(PROMPT_CACHE_TTL_SECONDS_PARAM, serde_json::json!(3600));

        let encoded = build_messages_request(&request).expect("request");
        let json: JsonValue = serde_json::from_str(&encoded).expect("json");
        assert_eq!(json["system"][0]["cache_control"]["type"], "ephemeral");
        assert_eq!(json["system"][0]["cache_control"]["ttl"], "1h");
        assert_eq!(
            json["messages"][2]["content"][0]["cache_control"]["type"],
            "ephemeral"
        );
        assert_eq!(
            json["messages"][3]["content"][0]["cache_control"]["type"],
            "ephemeral"
        );
    }

    #[test]
    fn prompt_cache_can_be_disabled_per_request() {
        let request = LlmRequest::new("anthropic/claude-sonnet-4")
            .message(Message::system("system instructions"))
            .message(Message::user("hello"))
            .extra_param(PROMPT_CACHE_PARAM, serde_json::json!("off"));
        let encoded = build_messages_request(&request).expect("request");
        let json: JsonValue = serde_json::from_str(&encoded).expect("json");
        assert!(
            json["system"][0].get("cache_control").is_none(),
            "cache_control should be omitted when prompt_cache=off: {json}"
        );
        assert!(
            json["messages"][0]["content"][0]
                .get("cache_control")
                .is_none(),
            "cache_control should be omitted when prompt_cache=off: {json}"
        );
    }

    #[tokio::test]
    async fn normalizer_emits_tool_use_and_usage() {
        let stream = futures_util::stream::iter(vec![
            Ok::<Bytes, LlmError>(Bytes::from(
                "data: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":3,\"cache_creation_input_tokens\":6}}}\n\n",
            )),
            Ok::<Bytes, LlmError>(Bytes::from(
                "data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"tool_use\",\"id\":\"call_1\",\"name\":\"read\",\"input\":{}}}\n\n",
            )),
            Ok::<Bytes, LlmError>(Bytes::from(
                "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{\\\"path\\\":\\\"Cargo.toml\\\"}\"}}\n\n",
            )),
            Ok::<Bytes, LlmError>(Bytes::from(
                "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"tool_use\"},\"usage\":{\"output_tokens\":2,\"cache_read_input_tokens\":4}}\n\n",
            )),
            Ok::<Bytes, LlmError>(Bytes::from("data: [DONE]\n\n")),
        ]);
        let bytes: crate::transport::ByteStream = Box::pin(stream);
        let frames = Box::pin(sse_data_frames(bytes, Duration::from_secs(1)));
        let events: Vec<LlmEvent> = normalize_messages_sse(frames).collect().await;
        assert!(
            events.iter().any(
                |event| matches!(event, LlmEvent::ToolCall(call) if call.id == "call_1" && call.name == "read")
            )
        );
        assert!(
            events
                .iter()
                .any(|event| matches!(event, LlmEvent::Usage(usage) if usage.total_tokens == 5))
        );
        assert!(events.iter().any(|event| matches!(
            event,
            LlmEvent::Usage(usage)
                if usage.cache_creation_input_tokens == Some(6)
                    && usage.cache_read_input_tokens == Some(4)
        )));
        assert_eq!(
            events.last(),
            Some(&LlmEvent::Done {
                reason: FinishReason::ToolCalls
            })
        );
    }
}
