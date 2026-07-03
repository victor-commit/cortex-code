//! Shared subagent/`task` delegation runner used by BOTH the headless `cortex run` CLI and the
//! live TUI turn.
//!
//! The `task` tool (in the `tools` crate) is a thin surface seam that calls a
//! [`tools::TaskRunner`]; the real durable child turn — session creation, prompt/persona assembly,
//! permission narrowing, child tool materializing, checkpoint/formatter/LSP wiring, and exact-PID
//! cleanup of any processes the child tools spawn — lives here.
//!
//! This logic was promoted out of the `cli` crate so the `tui` crate can advertise and execute the
//! `task` tool without a `cli <-> tui` dependency cycle: `cli` already depends on `tui` (it launches
//! the TUI), so `tui` cannot depend on `cli`. Both crates depend on THIS crate instead, and both
//! `cortex run` and a live TUI turn drive the SAME runner, so headless subagent behavior is
//! unchanged.

pub mod custom_agents;

mod bridges;
mod catalog;
mod provider;
mod runner;

#[cfg(test)]
mod tests;

pub use catalog::{SubagentCatalog, permitted_catalog};
pub use provider::{ChildProviderWrapper, ProviderAuditConfig};
pub use runner::{CliTaskRunner, CliTaskRunnerConfig};
