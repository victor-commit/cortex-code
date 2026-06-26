//! The OpenAI-compatible Chat-completions wire types: request serialization and streamed-chunk
//! deserialization. These are private to the adapter; the rest of the crate speaks the canonical
//! [`crate::LlmRequest`]/[`crate::LlmEvent`] vocabulary.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::error::LlmError;
use crate::event::Usage;
use crate::media::{encode_media_part, is_image_part, strip_media_stub};
use crate::request::{LlmRequest, Message, MessageContentPart, Role, ToolDef};

/// Serialize a canonical [`LlmRequest`] into a streaming Chat-completions request body.
///
/// `include_usage` adds `stream_options.include_usage` so the provider reports token usage in the
/// final chunk (OpenRouter honors this).
pub fn build_chat_request(
    request: &LlmRequest,
    include_usage: bool,
    provider_name: &str,
) -> Result<String, LlmError> {
    let body = ChatRequest {
        model: &request.model,
        messages: request.messages.iter().map(WireMessage::from).collect(),
        stream: true,
        max_tokens: request.max_tokens,
        temperature: request.temperature,
        tools: request
            .tools
            .iter()
            .map(|tool| WireTool::from_tool(tool, provider_name))
            .collect(),
        stream_options: include_usage.then_some(StreamOptions {
            include_usage: true,
        }),
    };
    let mut value = serde_json::to_value(&body)
        .map_err(|err| LlmError::transport(format!("failed to encode request: {err}")))?;
    if let serde_json::Value::Object(map) = &mut value {
        for (name, param) in &request.extra_params {
            if !is_reserved_chat_param(name) {
                map.insert(name.clone(), param.clone());
            }
        }
    }
    serde_json::to_string(&value)
        .map_err(|err| LlmError::transport(format!("failed to encode request: {err}")))
}

fn is_reserved_chat_param(name: &str) -> bool {
    matches!(
        name,
        "model" | "messages" | "tools" | "stream" | "stream_options"
    )
}

#[derive(Serialize)]
struct ChatRequest<'a> {
    model: &'a str,
    messages: Vec<WireMessage<'a>>,
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tools: Vec<WireTool<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stream_options: Option<StreamOptions>,
}

#[derive(Serialize)]
struct StreamOptions {
    include_usage: bool,
}

#[derive(Serialize)]
struct WireMessage<'a> {
    role: &'a str,
    content: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_call_id: Option<&'a str>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tool_calls: Vec<WireToolCall<'a>>,
}

impl<'a> From<&'a Message> for WireMessage<'a> {
    fn from(message: &'a Message) -> Self {
        WireMessage {
            role: role_str(message.role),
            content: wire_content(message),
            tool_call_id: message.tool_call_id.as_deref(),
            tool_calls: message
                .tool_calls
                .iter()
                .map(|call| WireToolCall {
                    id: &call.id,
                    r#type: "function",
                    function: WireFunctionCall {
                        name: &call.name,
                        arguments: &call.arguments,
                    },
                })
                .collect(),
        }
    }
}

fn wire_content(message: &Message) -> serde_json::Value {
    if message.parts.is_empty() {
        return serde_json::Value::String(message.content.clone());
    }

    let mut parts = Vec::new();
    if !message.content.is_empty() {
        parts.push(serde_json::json!({
            "type": "text",
            "text": message.content,
        }));
    }

    let allow_media = role_accepts_media(message.role);
    let mut strip_stubs = Vec::new();
    for part in &message.parts {
        match openai_media_part(part, allow_media) {
            Some(media_part) => parts.push(media_part),
            None => strip_stubs.push(strip_media_stub(part)),
        }
    }

    if !strip_stubs.is_empty() {
        parts.push(serde_json::json!({
            "type": "text",
            "text": strip_stubs.join("\n"),
        }));
    }

    serde_json::Value::Array(parts)
}

fn role_accepts_media(role: Role) -> bool {
    matches!(role, Role::User | Role::Tool)
}

fn openai_media_part(part: &MessageContentPart, allow_media: bool) -> Option<serde_json::Value> {
    if !allow_media || !is_image_part(part) {
        return None;
    }
    let encoded = encode_media_part(part).ok()?;
    Some(serde_json::json!({
        "type": "image_url",
        "image_url": {
            "url": format!("data:{};base64,{}", encoded.media_type, encoded.base64),
        }
    }))
}

#[derive(Serialize)]
struct WireToolCall<'a> {
    id: &'a str,
    r#type: &'a str,
    function: WireFunctionCall<'a>,
}

#[derive(Serialize)]
struct WireFunctionCall<'a> {
    name: &'a str,
    arguments: &'a str,
}

#[derive(Serialize)]
struct WireTool<'a> {
    r#type: &'a str,
    function: WireFunction<'a>,
}

#[derive(Serialize)]
struct WireFunction<'a> {
    name: &'a str,
    description: &'a str,
    parameters: serde_json::Value,
}

impl<'a> WireTool<'a> {
    fn from_tool(tool: &'a ToolDef, provider_name: &str) -> Self {
        WireTool {
            r#type: "function",
            function: WireFunction {
                name: &tool.name,
                description: &tool.description,
                parameters: shape_tool_parameters(&tool.parameters, provider_name),
            },
        }
    }
}

fn shape_tool_parameters(parameters: &serde_json::Value, provider_name: &str) -> serde_json::Value {
    // Some live OpenRouter routes intermittently reject tool schemas that carry a top-level
    // `anyOf`. Keep the local contract schema intact for deterministic validation, but reshape the
    // wire payload to the provider's accepted subset.
    if !provider_name.eq_ignore_ascii_case("openrouter") {
        return parameters.clone();
    }

    let mut shaped = parameters.clone();
    let Some(object) = shaped.as_object_mut() else {
        return shaped;
    };

    let Some(any_of) = object.remove("anyOf") else {
        return shaped;
    };
    let Some(any_of_branches) = any_of.as_array() else {
        object.insert("anyOf".to_owned(), any_of);
        return shaped;
    };

    if !object.contains_key("required")
        && let Some(required) = preferred_required_branch(any_of_branches, object.get("properties"))
    {
        object.insert("required".to_owned(), required);
    }

    shaped
}

fn preferred_required_branch(
    branches: &[serde_json::Value],
    properties: Option<&serde_json::Value>,
) -> Option<serde_json::Value> {
    let alias_keys = alias_property_keys(properties);
    let canonical_branches = branches
        .iter()
        .filter_map(required_branch_keys)
        .filter(|required| {
            required
                .iter()
                .all(|key| !alias_keys.contains(key.as_str()))
        })
        .collect::<Vec<_>>();
    if canonical_branches.len() == 1 {
        Some(serde_json::json!(canonical_branches[0]))
    } else {
        None
    }
}

fn required_branch_keys(branch: &serde_json::Value) -> Option<Vec<String>> {
    let required = branch.get("required")?.as_array()?;
    let mut keys = Vec::with_capacity(required.len());
    for key in required {
        keys.push(key.as_str()?.to_owned());
    }
    Some(keys)
}

fn alias_property_keys(properties: Option<&serde_json::Value>) -> HashSet<String> {
    let mut keys = HashSet::new();
    let Some(properties) = properties.and_then(serde_json::Value::as_object) else {
        return keys;
    };
    for (name, schema) in properties {
        let is_alias = schema
            .get("description")
            .and_then(serde_json::Value::as_str)
            .map(|description| description.contains("Compatibility alias"))
            .unwrap_or(false);
        if is_alias {
            keys.insert(name.clone());
        }
    }
    keys
}

const fn role_str(role: Role) -> &'static str {
    match role {
        Role::System => "system",
        Role::User => "user",
        Role::Assistant => "assistant",
        Role::Tool => "tool",
    }
}

/// One `chat.completion.chunk` from the streamed response.
#[derive(Deserialize)]
pub struct ChatChunk {
    #[serde(default)]
    pub choices: Vec<ChatChoice>,
    #[serde(default)]
    pub usage: Option<ChatUsage>,
    #[serde(default)]
    pub error: Option<ChatError>,
}

#[derive(Deserialize)]
pub struct ChatChoice {
    #[serde(default)]
    pub delta: ChatDelta,
    #[serde(default)]
    pub finish_reason: Option<String>,
}

#[derive(Deserialize, Default)]
pub struct ChatDelta {
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default)]
    pub reasoning: Option<String>,
    #[serde(default)]
    pub reasoning_content: Option<String>,
    #[serde(default)]
    pub tool_calls: Vec<ToolCallDelta>,
}

impl ChatDelta {
    /// The reasoning fragment from whichever reasoning field the provider populated.
    pub fn reasoning_text(&self) -> Option<String> {
        self.reasoning
            .clone()
            .or_else(|| self.reasoning_content.clone())
    }
}

#[derive(Deserialize)]
pub struct ToolCallDelta {
    #[serde(default)]
    pub index: usize,
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub function: Option<FunctionDelta>,
}

#[derive(Deserialize)]
pub struct FunctionDelta {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub arguments: Option<String>,
}

#[derive(Deserialize)]
pub struct ChatUsage {
    #[serde(default)]
    prompt_tokens: u32,
    #[serde(default)]
    completion_tokens: u32,
    #[serde(default)]
    total_tokens: u32,
    #[serde(default)]
    cost: Option<f64>,
}

impl ChatUsage {
    /// Convert provider token counts into the canonical [`Usage`].
    pub fn into_usage(self) -> Usage {
        Usage {
            input_tokens: self.prompt_tokens,
            output_tokens: self.completion_tokens,
            total_tokens: self.total_tokens,
            cost: self.cost,
        }
    }
}

#[derive(Deserialize)]
pub struct ChatError {
    #[serde(default)]
    message: String,
    #[serde(default)]
    code: Option<serde_json::Value>,
}

impl ChatError {
    /// A human-readable description of the upstream error.
    pub fn message(&self) -> String {
        match &self.code {
            Some(code) if !self.message.is_empty() => format!("{} (code {code})", self.message),
            Some(code) => format!("upstream error (code {code})"),
            None if !self.message.is_empty() => self.message.clone(),
            None => "unspecified upstream error".to_owned(),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    fn temp_file(name: &str, bytes: &[u8]) -> PathBuf {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "cortex-openai-wire-{name}-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        fs::write(&path, bytes).expect("write temp attachment");
        path
    }

    #[test]
    fn request_body_marks_streaming_and_includes_usage_option() {
        let request = LlmRequest::new("google/gemini-2.5-flash")
            .system("be terse")
            .user("hi");
        let body = build_chat_request(&request, true, "openrouter").expect("encode");
        let value: serde_json::Value = serde_json::from_str(&body).expect("json");
        assert_eq!(value["model"], "google/gemini-2.5-flash");
        assert_eq!(value["stream"], true);
        assert_eq!(value["stream_options"]["include_usage"], true);
        assert_eq!(value["messages"][0]["role"], "system");
        assert_eq!(value["messages"][1]["role"], "user");
        assert_eq!(value["messages"][1]["content"], "hi");
    }

    #[test]
    fn tools_serialize_as_function_specs() {
        let request = LlmRequest::new("m").user("hi").tool(ToolDef {
            name: "get_weather".to_owned(),
            description: "look up weather".to_owned(),
            parameters: serde_json::json!({"type": "object"}),
        });
        let body = build_chat_request(&request, false, "openrouter").expect("encode");
        let value: serde_json::Value = serde_json::from_str(&body).expect("json");
        assert_eq!(value["tools"][0]["type"], "function");
        assert_eq!(value["tools"][0]["function"]["name"], "get_weather");
        assert!(value.get("stream_options").is_none());
    }

    #[test]
    fn extra_params_merge_into_the_chat_request() {
        let request = LlmRequest::new("m")
            .user("hi")
            .extra_param("temperature", serde_json::json!(0.123));
        let body = build_chat_request(&request, false, "openrouter").expect("encode");
        let value: serde_json::Value = serde_json::from_str(&body).expect("json");
        assert_eq!(value["temperature"], 0.123);
    }

    #[test]
    fn extra_params_do_not_override_structural_fields() {
        let request = LlmRequest::new("m")
            .user("hi")
            .extra_param("messages", serde_json::json!("bad"))
            .extra_param("tools", serde_json::json!("bad"));
        let body = build_chat_request(&request, false, "openrouter").expect("encode");
        let value: serde_json::Value = serde_json::from_str(&body).expect("json");
        assert!(value["messages"].is_array());
        assert!(value.get("tools").is_none());
    }

    #[test]
    fn openrouter_relaxes_top_level_any_of_and_keeps_canonical_required_branch() {
        let params = serde_json::json!({
            "type": "object",
            "properties": {
                "path": { "type": "string" },
                "filePath": {
                    "type": "string",
                    "description": "Compatibility alias for `path`."
                }
            },
            "anyOf": [
                { "required": ["path"] },
                { "required": ["filePath"] }
            ]
        });
        let request = LlmRequest::new("m").user("hi").tool(ToolDef {
            name: "read".to_owned(),
            description: "read".to_owned(),
            parameters: params.clone(),
        });
        let body = build_chat_request(&request, false, "openrouter").expect("encode");
        let value: serde_json::Value = serde_json::from_str(&body).expect("json");
        let wire_params = &value["tools"][0]["function"]["parameters"];
        assert!(
            wire_params.get("anyOf").is_none(),
            "openrouter wire schema should omit top-level anyOf: {wire_params}"
        );
        assert_eq!(wire_params["required"], serde_json::json!(["path"]));
        assert!(
            request.tools[0].parameters.get("anyOf").is_some(),
            "source schema must remain unchanged for local validation"
        );
    }

    #[test]
    fn openrouter_relaxes_ambiguous_any_of_without_forcing_required_fields() {
        let params = serde_json::json!({
            "type": "object",
            "properties": {
                "subagent_type": { "type": "string" },
                "subagentType": {
                    "type": "string",
                    "description": "Compatibility alias for `subagent_type`."
                },
                "prompt": { "type": "string" },
                "task_id": { "type": "string" },
                "taskId": {
                    "type": "string",
                    "description": "Compatibility alias for `task_id`."
                }
            },
            "anyOf": [
                { "required": ["subagent_type", "prompt"] },
                { "required": ["subagentType", "prompt"] },
                { "required": ["task_id"] },
                { "required": ["taskId"] }
            ]
        });
        let request = LlmRequest::new("m").user("hi").tool(ToolDef {
            name: "task".to_owned(),
            description: "task".to_owned(),
            parameters: params,
        });
        let body = build_chat_request(&request, false, "openrouter").expect("encode");
        let value: serde_json::Value = serde_json::from_str(&body).expect("json");
        let wire_params = &value["tools"][0]["function"]["parameters"];
        assert!(
            wire_params.get("anyOf").is_none(),
            "openrouter wire schema should omit top-level anyOf: {wire_params}"
        );
        assert!(
            wire_params.get("required").is_none(),
            "ambiguous canonical branches should not force one required set: {wire_params}"
        );
    }

    #[test]
    fn non_openrouter_requests_keep_any_of_intact() {
        let request = LlmRequest::new("m").user("hi").tool(ToolDef {
            name: "read".to_owned(),
            description: "read".to_owned(),
            parameters: serde_json::json!({
                "type": "object",
                "anyOf": [{ "required": ["path"] }]
            }),
        });
        let body = build_chat_request(&request, false, "custom-provider").expect("encode");
        let value: serde_json::Value = serde_json::from_str(&body).expect("json");
        assert!(
            value["tools"][0]["function"]["parameters"]
                .get("anyOf")
                .is_some()
        );
    }

    #[test]
    fn user_image_parts_serialize_to_multimodal_content_array() {
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
        let request =
            LlmRequest::new("m").message(Message::user("read this image").with_parts(vec![
                MessageContentPart::Image {
                    media_type: "image/png".to_owned(),
                    path: image.to_string_lossy().into_owned(),
                },
            ]));
        let body = build_chat_request(&request, false, "openrouter").expect("encode");
        let value: serde_json::Value = serde_json::from_str(&body).expect("json");
        let content = value["messages"][0]["content"]
            .as_array()
            .expect("multimodal array content");
        assert_eq!(content[0]["type"], "text");
        assert_eq!(content[1]["type"], "image_url");
        assert!(
            content[1]["image_url"]["url"]
                .as_str()
                .is_some_and(|url| url.starts_with("data:image/png;base64,")),
            "expected data URL image part, got {}",
            content[1]
        );
    }

    #[test]
    fn unsupported_media_parts_fallback_to_strip_media_stub_text() {
        let doc = temp_file("doc.pdf", b"%PDF-1.7\n");
        let request =
            LlmRequest::new("m").message(Message::user("summarize attachment").with_parts(vec![
                MessageContentPart::Document {
                    media_type: "application/pdf".to_owned(),
                    path: doc.to_string_lossy().into_owned(),
                },
            ]));
        let body = build_chat_request(&request, false, "openrouter").expect("encode");
        let value: serde_json::Value = serde_json::from_str(&body).expect("json");
        let content = value["messages"][0]["content"]
            .as_array()
            .expect("multimodal array");
        assert_eq!(content[1]["type"], "text");
        assert_eq!(content[1]["text"], "[Attached application/pdf]");
    }

    #[test]
    fn tool_role_messages_can_carry_image_parts() {
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
        let message = Message::tool_result("call_1", "tool produced an image").with_parts(vec![
            MessageContentPart::Image {
                media_type: "image/png".to_owned(),
                path: image.to_string_lossy().into_owned(),
            },
        ]);
        let request = LlmRequest::new("m").message(message);
        let body = build_chat_request(&request, false, "openrouter").expect("encode");
        let value: serde_json::Value = serde_json::from_str(&body).expect("json");
        assert_eq!(value["messages"][0]["role"], "tool");
        assert_eq!(value["messages"][0]["tool_call_id"], "call_1");
        assert_eq!(value["messages"][0]["content"][1]["type"], "image_url");
    }

    #[test]
    fn chunk_with_reasoning_field_is_parsed() {
        let chunk: ChatChunk =
            serde_json::from_str(r#"{"choices":[{"delta":{"reasoning":"thinking"}}]}"#)
                .expect("parse");
        assert_eq!(
            chunk.choices[0].delta.reasoning_text(),
            Some("thinking".to_owned())
        );
    }
}
