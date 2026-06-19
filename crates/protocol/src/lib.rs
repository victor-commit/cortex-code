//! Wire/DTO types and protocol vocabulary shared across Cortex Code crates.

mod todo;
mod tool;

pub use todo::{TodoItem, TodoStatus};
pub use tool::ToolResultPart;
