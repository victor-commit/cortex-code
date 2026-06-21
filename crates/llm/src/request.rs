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
            tool_call_id: Some(tool_call_id.into()),
            tool_calls: Vec::new(),
        }
    }

    fn text(role: Role, content: impl Into<String>) -> Self {
        Self {
            role,
            content: content.into(),
            tool_call_id: None,
            tool_calls: Vec::new(),
        }
    }
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
}
