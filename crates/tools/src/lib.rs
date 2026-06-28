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
mod bash;
mod edit;
mod glob;
mod grep;
mod invalid;
mod loop_safeguard;
mod lsp;
mod output;
mod plan;
mod plan_enter;
mod plan_exit;
mod plan_exit_compat;
mod question;
mod read;
mod registry;
mod ripgrep;
mod skill;
mod task;
mod todo;
mod tool;
mod webfetch;
mod websearch;
mod write;

pub use apply_patch::{ApplyPatchTool, PatchFileDiff, patch_file_diffs};
pub use bash::{BackgroundJob, BackgroundJobs, BashTool, JobHandle};
pub use edit::EditTool;
pub use glob::GlobTool;
pub use grep::GrepTool;
pub use invalid::{InvalidTool, invalid_arguments_output, unknown_tool_output};
pub use loop_safeguard::LoopSafeguardTool;
pub use lsp::LspTool;
pub use output::{BoundedOutput, DEFAULT_MAX_BYTES, DEFAULT_MAX_LINES, OutputLimits, bound_output};
pub use plan::PlanTool;
pub use plan_enter::PlanEnterTool;
pub use plan_exit::PlanExitTool;
pub use plan_exit_compat::PlanExitCompatTool;
pub use question::QuestionTool;
pub use read::{DEFAULT_READ_LIMIT, ReadTool};
pub use registry::ToolRegistry;
pub use skill::SkillTool;
pub use task::TaskTool;
pub use todo::TodoTool;
pub use tool::{
    CheckpointRecorder, Diagnostic, DiagnosticFeedback, LspOperation, LspRequest, LspService,
    MutationHooks, TaskRequest, TaskRunner, TaskTarget, TodoStore, Tool, ToolContext, ToolOutput,
};
pub use webfetch::WebFetchTool;
pub use websearch::{WebSearchConfig, WebSearchProvider, WebSearchTool};
pub use write::WriteTool;
