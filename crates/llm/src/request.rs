//! The provider-agnostic request a [`Provider`](crate::Provider) turns into a streamed turn.
//!
//! [`LlmRequest`] is the canonical input: a model id, an ordered conversation of [`Message`]s, and
//! the optional tool catalogue the model may call. Each adapter translates it into its own wire
//! format; the runner builds it without knowing which protocol will carry it.

use serde::{Deserialize, Serialize};
use serde_json::{Map as JsonMap, Value as JsonValue};

use crate::event::ToolCall;

/// A speaker role in the conversation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    /// System/developer instructions.
    System,
    /// An end-user message.
    User,
    /// A prior assistant message (text and/or tool calls).
    Assistant,
    /// A tool result addressed to a prior [`ToolCall`].
    Tool,
}

/// One message in the conversation history sent to the model.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Message {
    /// Who is speaking.
    pub role: Role,
    /// The text content of the message (may be empty for an assistant turn that only made tool
    /// calls).
    pub content: String,
    /// Optional non-text media parts associated with this message.
    ///
    /// Provider adapters serialize these into native multimodal payloads when supported; when a
    /// provider or message role cannot carry binary media they are reduced to safe
    /// `[Attached <media-type>]` text stubs.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub parts: Vec<MessageContentPart>,
    /// For [`Role::Tool`] messages: the [`ToolCall::id`] this result answers.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    /// For [`Role::Assistant`] messages replayed from history: the tool calls the assistant made.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub tool_calls: Vec<ToolCall>,
}

impl Message {
    /// A system/developer instruction message.
    #[must_use]
    pub fn system(content: impl Into<String>) -> Self {
        Self::text(Role::System, content)
    }

    /// A user message.
    #[must_use]
    pub fn user(content: impl Into<String>) -> Self {
        Self::text(Role::User, content)
    }

    /// An assistant text message.
    #[must_use]
    pub fn assistant(content: impl Into<String>) -> Self {
        Self::text(Role::Assistant, content)
    }

    /// A tool-result message answering a prior [`ToolCall`].
    #[must_use]
    pub fn tool_result(tool_call_id: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            role: Role::Tool,
            content: content.into(),
            parts: Vec::new(),
            tool_call_id: Some(tool_call_id.into()),
            tool_calls: Vec::new(),
        }
    }

    /// Attach non-text media parts to this message (builder style).
    #[must_use]
    pub fn with_parts(mut self, parts: Vec<MessageContentPart>) -> Self {
        self.parts = parts;
        self
    }

    fn text(role: Role, content: impl Into<String>) -> Self {
        Self {
            role,
            content: content.into(),
            parts: Vec::new(),
            tool_call_id: None,
            tool_calls: Vec::new(),
        }
    }
}

/// A typed non-text content part attached to a message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MessageContentPart {
    /// An image attachment (for example `image/png`).
    Image { media_type: String, path: String },
    /// A document attachment (for example `application/pdf`).
    Document { media_type: String, path: String },
    /// A generic file attachment.
    File {
        path: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        media_type: Option<String>,
    },
    /// Provider-specific reasoning metadata needed to safely replay prior thinking context.
    Reasoning {
        /// Provider id this reasoning metadata belongs to (for example `anthropic` or `gemini`).
        provider: String,
        /// The non-user-visible reasoning text.
        text: String,
        /// Optional provider-issued signature/token needed to replay the reasoning block.
        #[serde(skip_serializing_if = "Option::is_none")]
        signature: Option<String>,
    },
}

/// Canonical tool-choice modes normalized across provider adapters.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ToolChoice {
    /// Let the model decide whether to call tools.
    Auto,
    /// Do not allow tool calls.
    None,
    /// Require at least one tool call.
    Required,
    /// Require a specific tool by name.
    Tool { name: String },
}

/// A tool the model is permitted to call, described by a JSON-schema parameter definition.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolDef {
    /// The tool/function name.
    pub name: String,
    /// A natural-language description of when/how to use the tool.
    pub description: String,
    /// The JSON Schema describing the tool's arguments object.
    pub parameters: serde_json::Value,
}

/// A complete, provider-agnostic streaming request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LlmRequest {
    /// The model identifier (e.g. `google/gemini-2.5-flash`).
    pub model: String,
    /// The ordered conversation history.
    pub messages: Vec<Message>,
    /// The tool catalogue offered to the model for this turn.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub tools: Vec<ToolDef>,
    /// Optional tool-choice mode for providers that support function/tool calling controls.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<ToolChoice>,
    /// Optional cap on generated tokens.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
    /// Optional sampling temperature.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    /// Provider-specific Chat Completions parameters applied after the canonical fields are
    /// serialized. This lets surfaces add request-scoped options such as `top_p` without changing
    /// the provider-agnostic runner API for every adapter.
    #[serde(skip_serializing_if = "JsonMap::is_empty", default)]
    pub extra_params: JsonMap<String, JsonValue>,
    /// Request-scoped HTTP headers merged after provider-level headers so callers can inject or
    /// override headers on a single outbound request.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub extra_headers: Vec<(String, String)>,
}

impl LlmRequest {
    /// Start a request for `model` with an empty conversation.
    #[must_use]
    pub fn new(model: impl Into<String>) -> Self {
        Self {
            model: model.into(),
            messages: Vec::new(),
            tools: Vec::new(),
            tool_choice: None,
            max_tokens: None,
            temperature: None,
            extra_params: JsonMap::new(),
            extra_headers: Vec::new(),
        }
    }

    /// Append a message (builder style).
    #[must_use]
    pub fn message(mut self, message: Message) -> Self {
        self.messages.push(message);
        self
    }

    /// Append a system message (builder style).
    #[must_use]
    pub fn system(self, content: impl Into<String>) -> Self {
        self.message(Message::system(content))
    }

    /// Append a user message (builder style).
    #[must_use]
    pub fn user(self, content: impl Into<String>) -> Self {
        self.message(Message::user(content))
    }

    /// Register a callable tool (builder style).
    #[must_use]
    pub fn tool(mut self, tool: ToolDef) -> Self {
        self.tools.push(tool);
        self
    }

    /// Set a normalized tool-choice mode (builder style).
    #[must_use]
    pub fn tool_choice(mut self, tool_choice: ToolChoice) -> Self {
        self.tool_choice = Some(tool_choice);
        self
    }

    /// Set the max-tokens cap (builder style).
    #[must_use]
    pub fn max_tokens(mut self, max_tokens: u32) -> Self {
        self.max_tokens = Some(max_tokens);
        self
    }

    /// Set the sampling temperature (builder style).
    #[must_use]
    pub fn temperature(mut self, temperature: f32) -> Self {
        self.temperature = Some(temperature);
        self
    }

    /// Set an adapter-specific request parameter (builder style).
    #[must_use]
    pub fn extra_param(mut self, name: impl Into<String>, value: JsonValue) -> Self {
        self.extra_params.insert(name.into(), value);
        self
    }

    /// Add a request-scoped HTTP header (builder style).
    #[must_use]
    pub fn extra_header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.extra_headers.push((name.into(), value.into()));
        self
    }

    /// Normalize known canonical options from `extra_params` into dedicated request fields.
    ///
    /// This currently extracts `tool_choice`/`toolChoice` when the typed field is still unset.
    pub fn normalize_known_params(&mut self) {
        if self.tool_choice.is_none() {
            self.tool_choice = take_tool_choice_from_extra_params(&mut self.extra_params);
        }
    }
}

fn take_tool_choice_from_extra_params(
    params: &mut JsonMap<String, JsonValue>,
) -> Option<ToolChoice> {
    let raw = params
        .remove("tool_choice")
        .or_else(|| params.remove("toolChoice"))?;
    parse_tool_choice_value(&raw)
}

fn parse_tool_choice_value(raw: &JsonValue) -> Option<ToolChoice> {
    match raw {
        JsonValue::String(value) => parse_tool_choice_string(value),
        JsonValue::Object(object) => {
            let kind = object
                .get("type")
                .or_else(|| object.get("mode"))
                .and_then(JsonValue::as_str);
            match kind.and_then(parse_tool_choice_string) {
                Some(ToolChoice::Tool { .. }) => {
                    let name = object
                        .get("name")
                        .or_else(|| object.get("tool"))
                        .or_else(|| object.get("tool_name"))
                        .or_else(|| {
                            object
                                .get("function")
                                .and_then(JsonValue::as_object)
                                .and_then(|function| function.get("name"))
                        })
                        .and_then(JsonValue::as_str)?;
                    Some(ToolChoice::Tool {
                        name: name.to_owned(),
                    })
                }
                Some(choice) => Some(choice),
                None => object
                    .get("name")
                    .or_else(|| object.get("tool"))
                    .and_then(JsonValue::as_str)
                    .map(|name| ToolChoice::Tool {
                        name: name.to_owned(),
                    }),
            }
        }
        _ => None,
    }
}

fn parse_tool_choice_string(value: &str) -> Option<ToolChoice> {
    match value.trim().to_ascii_lowercase().as_str() {
        "auto" => Some(ToolChoice::Auto),
        "none" | "off" | "disabled" => Some(ToolChoice::None),
        "required" | "any" => Some(ToolChoice::Required),
        "tool" | "function" => Some(ToolChoice::Tool {
            name: String::new(),
        }),
        _ => Some(ToolChoice::Tool {
            name: value.trim().to_owned(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builder_accumulates_messages_and_tools() {
        let request = LlmRequest::new("google/gemini-2.5-flash")
            .system("be terse")
            .user("hi")
            .max_tokens(64);
        assert_eq!(request.model, "google/gemini-2.5-flash");
        assert_eq!(request.messages.len(), 2);
        assert_eq!(request.messages[0].role, Role::System);
        assert_eq!(request.messages[1].role, Role::User);
        assert_eq!(request.max_tokens, Some(64));
    }

    #[test]
    fn tool_result_carries_its_call_id() {
        let message = Message::tool_result("call_7", "42");
        assert_eq!(message.role, Role::Tool);
        assert_eq!(message.tool_call_id.as_deref(), Some("call_7"));
        assert_eq!(message.content, "42");
    }

    #[test]
    fn normalize_known_params_extracts_tool_choice() {
        let mut request = LlmRequest::new("google/gemini-2.5-flash").extra_param(
            "tool_choice",
            serde_json::json!({"type":"tool","name":"todo"}),
        );
        request.normalize_known_params();
        assert_eq!(
            request.tool_choice,
            Some(ToolChoice::Tool {
                name: "todo".to_owned()
            })
        );
        assert!(
            !request.extra_params.contains_key("tool_choice"),
            "tool_choice should move into the typed request field"
        );
    }
}
