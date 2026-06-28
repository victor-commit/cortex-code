//! The LLM provider/transport foundation for Cortex Code.
//!
//! This crate is the plumbing the runner and CLI consume to talk to a model. It defines:
//!
//! - the canonical normalized [`LlmEvent`] union every adapter emits (text/reasoning deltas,
//!   assembled tool calls, tool results, usage, errors, and a terminal `Done`);
//! - the object-safe [`Provider`] trait that streams a [`LlmRequest`] into an [`LlmStream`];
//! - a rustls-only [`HttpTransport`] with SSE streaming (via `eventsource-stream`) and a per-frame
//!   idle timeout, fronted by the `CORTEX_NO_NET` outbound guard so the kill-switch fails closed
//!   before any socket opens;
//! - the OpenAI-compatible Chat-completions adapter ([`OpenAiCompatible`]) — the reference path used
//!   for OpenRouter — which normalizes provider SSE bytes into the [`LlmEvent`] stream; and
//! - normalization of every provider/transport failure into the canonical [`LlmError`] classes
//!   (distinct `net-disabled` / `transport` / `upstream`).
//!
//! There are no mocks in the shipped path: the live transport and the offline cassette tests run
//! the exact same normalization code, differing only in where the SSE bytes come from.

mod anthropic;
mod error;
mod event;
mod gemini;
mod media;
mod openai;
mod provider;
mod request;
mod transport;

pub use anthropic::{ANTHROPIC_API_BASE, AnthropicConfig, AnthropicMessages};
pub use error::LlmError;
pub use event::{FinishReason, LlmEvent, ToolCall, ToolResult, Usage};
pub use gemini::{GEMINI_API_BASE, GeminiConfig, GeminiProvider};
pub use media::{
    ImageNormalization, MAX_IMAGE_BYTES, MAX_IMAGE_DIMENSION, is_normalizable_image,
    normalize_image_bytes,
};
pub use openai::FrameStream;
pub use openai::{
    OPENROUTER_BASE_URL, OpenAiCompatible, OpenAiConfig, OpenAiResponses, OpenAiResponsesConfig,
    normalize_chat_sse,
};
pub use provider::{LlmStream, Provider, RequestMiddleware, with_request_middlewares};
pub use request::{LlmRequest, Message, MessageContentPart, Role, ToolChoice, ToolDef};
pub use transport::{
    ByteStream, HttpTransport, TransportConfig, classify_upstream, sse_data_frames,
};
