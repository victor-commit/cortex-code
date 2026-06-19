//! Cortex Code agent tools (architecture §3.4).
//!
//! This crate hosts the agent's file/shell/search tools. Each tool implements the [`Tool`] trait —
//! a name, a model-facing description, a JSON-Schema parameter object, and an async execution that
//! produces a structured [`ToolOutput`] (text plus any media attachments). The [`ToolRegistry`]
//! owns the registered tools, projects them into the provider-agnostic [`llm::ToolDef`] catalogue
//! the runner advertises, and dispatches an assembled tool call to its implementation.
//!
//! The runner stays decoupled from concrete tools: it calls the `engine` crate's `ToolDispatcher`
//! seam, which a surface (the CLI) implements by bridging to a [`ToolRegistry`]. Permission gating,
//! the per-session read-tracking ledger, and path resolution are carried by the [`ToolContext`]
//! shared across a turn's tool calls.

mod apply_patch;
mod edit;
mod glob;
mod grep;
mod read;
mod registry;
mod ripgrep;
mod todo;
mod tool;
mod write;

pub use apply_patch::ApplyPatchTool;
pub use edit::EditTool;
pub use glob::GlobTool;
pub use grep::GrepTool;
pub use read::{DEFAULT_READ_LIMIT, ReadTool};
pub use registry::ToolRegistry;
pub use todo::TodoTool;
pub use tool::{Diagnostic, MutationHooks, TodoStore, Tool, ToolContext, ToolOutput};
pub use write::WriteTool;
