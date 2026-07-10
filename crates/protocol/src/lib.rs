//! Wire/DTO types and protocol vocabulary shared across Cortex Code crates.

mod message;
mod todo;
mod tool;

pub use message::{MessagePart, MessagePartBody, StepPartPhase, ToolPartState};
pub use todo::{TodoItem, TodoPriority, TodoStatus};
pub use tool::{
    McpContentBlock, McpEmbeddedResource, McpPayload, McpSpillReference, McpStructuredContent,
    McpToolResult, ToolResultPart,
};
