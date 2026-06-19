//! Cortex Code MCP client (architecture §3.4).
//!
//! This crate connects to Model Context Protocol servers via the `rmcp` 1.7 SDK and exposes their
//! tools to the agent. A local (stdio) server declared in config is spawned as a child process
//! parented under cortex; the client completes the MCP initialize handshake (protocol version +
//! capability exchange), issues `tools/list` to populate the session catalogue, and exposes every
//! tool under a deterministic, sanitized, server-qualified id so two servers exporting the same tool
//! name never clash. A real `tools/call` made during an agent turn reaches the server and its
//! genuine result (text and/or structured content) is relayed faithfully back into the turn, with
//! oversized output truncated to a spill file rather than silently lost. On shutdown every spawned
//! server is terminated by cancelling its session, so no orphan process survives.
//!
//! - [`McpManager`] connects every configured server, owns the qualified tool catalogue, routes a
//!   call to its owning server, and shuts every server down.
//! - [`McpClient`] is one connected stdio session: handshake, `tools/list`, `tools/call`, shutdown.
//! - [`servers_from_value`] parses the merged `mcp` config table into typed server entries.

mod client;
mod config;
mod error;
mod manager;
mod qualify;
mod result;

pub use client::{DEFAULT_TIMEOUT, McpClient};
pub use config::{McpConfigError, McpServerConfig, McpTransport, servers_from_value};
pub use error::McpError;
pub use manager::{ConnectionState, McpManager, McpTool, ServerStatus};
pub use qualify::qualified_id;
pub use result::{DEFAULT_MAX_RELAY_BYTES, RelayedResult, relay, relay_with_limit};
