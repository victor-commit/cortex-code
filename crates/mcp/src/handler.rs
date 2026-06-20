//! The client-side MCP notification handler that keeps the live tool catalogue in sync.
//!
//! Every connected session is served with a [`NotifyHandler`] rather than the unit handler, so a
//! server's `notifications/tools/list_changed` triggers an immediate `tools/list` re-fetch and a
//! catalogue update (which in turn publishes `mcp.tools.changed`). All other notifications keep the
//! default no-op behavior.

use std::time::Duration;

use rmcp::ClientHandler;
use rmcp::service::{NotificationContext, RoleClient};

use crate::catalogue::{LiveCatalogue, qualify_prompts, qualify_resources, qualify_tools};

/// A [`ClientHandler`] that re-lists a server's tools when it announces a tool-list change.
#[derive(Clone)]
pub(crate) struct NotifyHandler {
    server: String,
    catalogue: LiveCatalogue,
    timeout: Duration,
}

impl NotifyHandler {
    /// Build a handler for `server` that updates `catalogue` on list-changed notifications.
    pub(crate) fn new(server: String, catalogue: LiveCatalogue, timeout: Duration) -> Self {
        Self {
            server,
            catalogue,
            timeout,
        }
    }
}

impl ClientHandler for NotifyHandler {
    async fn on_tool_list_changed(&self, context: NotificationContext<RoleClient>) {
        match tokio::time::timeout(self.timeout, context.peer.list_all_tools()).await {
            Ok(Ok(tools)) => {
                let qualified = qualify_tools(&self.server, tools);
                self.catalogue
                    .update_tools_from_changed(&self.server, qualified);
            }
            Ok(Err(err)) => {
                tracing::warn!(
                    target: "cortex::events",
                    server = %self.server,
                    "failed to re-list MCP tools after a change notification: {err}"
                );
            }
            Err(_) => {
                tracing::warn!(
                    target: "cortex::events",
                    server = %self.server,
                    "timed out re-listing MCP tools after a change notification"
                );
            }
        }
    }

    async fn on_resource_list_changed(&self, context: NotificationContext<RoleClient>) {
        match tokio::time::timeout(self.timeout, context.peer.list_all_resources()).await {
            Ok(Ok(resources)) => {
                let qualified = qualify_resources(&self.server, resources);
                self.catalogue
                    .update_resources_from_changed(&self.server, qualified);
            }
            Ok(Err(err)) => {
                tracing::warn!(
                    target: "cortex::events",
                    server = %self.server,
                    "failed to re-list MCP resources after a change notification: {err}"
                );
            }
            Err(_) => {
                tracing::warn!(
                    target: "cortex::events",
                    server = %self.server,
                    "timed out re-listing MCP resources after a change notification"
                );
            }
        }
    }

    async fn on_prompt_list_changed(&self, context: NotificationContext<RoleClient>) {
        match tokio::time::timeout(self.timeout, context.peer.list_all_prompts()).await {
            Ok(Ok(prompts)) => {
                let qualified = qualify_prompts(&self.server, prompts);
                self.catalogue
                    .update_prompts_from_changed(&self.server, qualified);
            }
            Ok(Err(err)) => {
                tracing::warn!(
                    target: "cortex::events",
                    server = %self.server,
                    "failed to re-list MCP prompts after a change notification: {err}"
                );
            }
            Err(_) => {
                tracing::warn!(
                    target: "cortex::events",
                    server = %self.server,
                    "timed out re-listing MCP prompts after a change notification"
                );
            }
        }
    }
}
