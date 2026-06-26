//! The tool-materialization seam the runner fills with a real tool registry.
//!
//! The durable runner does not own a tool implementation (architecture §3.3/§4): it asks an
//! injected [`ToolDispatcher`] for the tool catalogue to advertise to the model and to execute each
//! tool call the model makes. The `tools` capability crate provides the real implementation and the
//! CLI bridges it in, so the runner stays decoupled from concrete tools (no `engine -> tools`
//! dependency). When no dispatcher is supplied the runner advertises no tools and answers directly.

use errors::Diagnostic;
use futures_util::future::BoxFuture;
use llm::{ToolCall, ToolDef};
use protocol::ToolResultPart;

/// The structured result of executing one tool call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolDispatchResult {
    /// The text re-injected into the conversation as the tool-result message the model reads back.
    pub content: String,
    /// The structured parts surfaced to machine consumers (text plus any attachments).
    pub parts: Vec<ToolResultPart>,
    /// Whether the tool reported an error. The turn still continues so the model can recover; this
    /// only marks the result so a consumer can distinguish a tool failure from a success.
    pub is_error: bool,
}

impl ToolDispatchResult {
    /// A successful result whose model-facing content is a single text part.
    #[must_use]
    pub fn text(content: impl Into<String>) -> Self {
        let content = content.into();
        Self {
            parts: vec![ToolResultPart::text(content.clone())],
            content,
            is_error: false,
        }
    }
}

/// The seam through which the runner materializes and executes tools.
///
/// [`definitions`](ToolDispatcher::definitions) returns the tool catalogue advertised to the model
/// for the turn; [`dispatch`](ToolDispatcher::dispatch) executes one assembled [`ToolCall`] and
/// returns its [`ToolDispatchResult`]. Implementations are owned outside `engine` (the `tools`
/// crate, bridged by the CLI) so the runner depends only on this trait.
pub trait ToolDispatcher: Send + Sync {
    /// The tool catalogue to advertise to the model this turn.
    fn definitions(&self) -> Vec<ToolDef>;

    /// Execute one assembled tool call, resolving to its structured result.
    fn dispatch<'a>(&'a self, call: &'a ToolCall) -> BoxFuture<'a, ToolDispatchResult>;

    /// Optional dynamic system-prompt instructions that should be appended before the next model
    /// request (for example, memory instructions activated by prior tool reads).
    ///
    /// Returning an error fails the turn closed before the next provider request.
    fn dynamic_system_instructions(&self) -> Result<Option<String>, Diagnostic> {
        Ok(None)
    }
}
