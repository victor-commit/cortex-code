//! The client-side MCP notification handler that keeps the live tool catalogue in sync.
//!
//! Every connected session is served with a [`NotifyHandler`] rather than the unit handler, so a
//! server's `notifications/tools/list_changed` triggers an immediate `tools/list` re-fetch and a
//! catalogue update (which in turn publishes `mcp.tools.changed`). All other notifications keep the
//! default no-op behavior.

use rmcp::ClientHandler;
use rmcp::service::{NotificationContext, RoleClient};

use crate::catalogue::{LiveCatalogue, qualify_tools};

/// A [`ClientHandler`] that re-lists a server's tools when it announces a tool-list change.
#[derive(Clone)]
pub(crate) struct NotifyHandler {
    server: String,
    catalogue: LiveCatalogue,
}

impl NotifyHandler {
    /// Build a handler for `server` that updates `catalogue` on a tool-list-changed notification.
    pub(crate) fn new(server: String, catalogue: LiveCatalogue) -> Self {
        Self { server, catalogue }
    }
}

impl ClientHandler for NotifyHandler {
    async fn on_tool_list_changed(&self, context: NotificationContext<RoleClient>) {
        match context.peer.list_all_tools().await {
            Ok(tools) => {
                let qualified = qualify_tools(&self.server, tools);
                self.catalogue.update_from_changed(&self.server, qualified);
            }
            Err(err) => {
                tracing::warn!(
                    target: "cortex::events",
                    server = %self.server,
                    "failed to re-list MCP tools after a change notification: {err}"
                );
            }
        }
    }
}
