//! Cortex Code MCP client (architecture §3.4).
//!
//! This crate connects to Model Context Protocol servers via the `rmcp` 1.7 SDK and exposes their
//! tools to the agent. A local (stdio) server declared in config is spawned as a child process
//! parented under cortex; a remote server declared with a `url` is reached over HTTP (Streamable
//! HTTP, falling back to the legacy HTTP+SSE transport) with optional headers and an OAuth bearer
//! token. For every transport the client completes the MCP initialize handshake, issues `tools/list`
//! to populate the session catalogue, and exposes each tool under a deterministic, sanitized,
//! server-qualified id. The catalogue is *live*: a server's tool-list-changed notification refreshes
//! its slice without a restart and publishes an `mcp.tools.changed` event. A real `tools/call`
//! during an agent turn reaches the server and its genuine result is relayed back. On shutdown every
//! spawned stdio server and its descendants are terminated by exact PID, so no orphan survives.
//!
//! - [`McpManager`] connects every configured server, owns the live qualified tool catalogue, routes
//!   a call to its owning server, and shuts every server down.
//! - [`McpClient`] is one connected session (stdio or remote): handshake, `tools/list`,
//!   `tools/call`, shutdown.
//! - [`servers_from_value`] parses the merged `mcp` config table into typed server entries.
//! - [`CredentialStore`] persists OAuth tokens to a `0600` store; [`oauth`] runs the authorization
//!   flow.

mod catalogue;
mod client;
mod config;
mod credentials;
mod error;
mod handler;
mod manager;
pub mod oauth;
mod process_tree;
mod qualify;
mod result;
mod transport_sse;

pub use catalogue::{
    LiveCatalogue, McpPrompt, McpPromptArgument, McpResource, McpServerCatalogue, McpTool,
    qualify_prompts, qualify_resources, qualify_tools,
};
pub use client::{DEFAULT_TIMEOUT, McpClient};
pub use config::{McpConfigError, McpServerConfig, McpTransport, servers_from_value};
pub use credentials::{CredentialStore, StoredToken, now_secs};
pub use error::McpError;
pub use manager::{ConnectionState, McpConnectOptions, McpManager, ServerStatus};
pub use process_tree::{pid_alive, process_tree};
pub use qualify::qualified_id;
pub use result::{
    DEFAULT_MAX_RELAY_BYTES, RelayedResult, relay, relay_get_prompt, relay_read_resource,
    relay_with_limit,
};
