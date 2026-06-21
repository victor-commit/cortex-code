//! The OpenAI-compatible Chat-completions wire types: request serialization and streamed-chunk
//! deserialization. These are private to the adapter; the rest of the crate speaks the canonical
//! [`crate::LlmRequest`]/[`crate::LlmEvent`] vocabulary.

use serde::{Deserialize, Serialize};

use crate::error::LlmError;
use crate::event::Usage;
use crate::request::{LlmRequest, Message, Role, ToolDef};

/// Serialize a canonical [`LlmRequest`] into a streaming Chat-completions request body.
///
/// `include_usage` adds `stream_options.include_usage` so the provider reports token usage in the
/// final chunk (OpenRouter honors this).
pub fn build_chat_request(request: &LlmRequest, include_usage: bool) -> Result<String, LlmError> {
    let body = ChatRequest {
        model: &request.model,
        messages: request.messages.iter().map(WireMessage::from).collect(),
        stream: true,
        max_tokens: request.max_tokens,
        temperature: request.temperature,
        tools: request.tools.iter().map(WireTool::from).collect(),
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
    content: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_call_id: Option<&'a str>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tool_calls: Vec<WireToolCall<'a>>,
}

impl<'a> From<&'a Message> for WireMessage<'a> {
    fn from(message: &'a Message) -> Self {
        WireMessage {
            role: role_str(message.role),
            content: &message.content,
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
    parameters: &'a serde_json::Value,
}

impl<'a> From<&'a ToolDef> for WireTool<'a> {
    fn from(tool: &'a ToolDef) -> Self {
        WireTool {
            r#type: "function",
            function: WireFunction {
                name: &tool.name,
                description: &tool.description,
                parameters: &tool.parameters,
            },
        }
    }
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
    use super::*;

    #[test]
    fn request_body_marks_streaming_and_includes_usage_option() {
        let request = LlmRequest::new("google/gemini-2.5-flash")
            .system("be terse")
            .user("hi");
        let body = build_chat_request(&request, true).expect("encode");
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
        let body = build_chat_request(&request, false).expect("encode");
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
        let body = build_chat_request(&request, false).expect("encode");
        let value: serde_json::Value = serde_json::from_str(&body).expect("json");
        assert_eq!(value["temperature"], 0.123);
    }

    #[test]
    fn extra_params_do_not_override_structural_fields() {
        let request = LlmRequest::new("m")
            .user("hi")
            .extra_param("messages", serde_json::json!("bad"))
            .extra_param("tools", serde_json::json!("bad"));
        let body = build_chat_request(&request, false).expect("encode");
        let value: serde_json::Value = serde_json::from_str(&body).expect("json");
        assert!(value["messages"].is_array());
        assert!(value.get("tools").is_none());
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
