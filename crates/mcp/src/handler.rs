//! The client-side MCP notification handler that keeps the live MCP bridge in sync.
//!
//! Every connected session is served with a [`NotifyHandler`] rather than the unit handler, so a
//! server's list-changed notifications trigger immediate re-lists (publishing typed MCP catalogue
//! change events), `notifications/message` log payloads are bridged onto the event bus as `mcp.log`,
//! and `roots/list` requests are answered from the configured project roots.

use std::sync::Arc;
use std::time::Duration;

use rmcp::ClientHandler;
use rmcp::model::{
    ClientCapabilities, ClientInfo, ListRootsResult, LoggingLevel, LoggingMessageNotificationParam,
};
use rmcp::service::{NotificationContext, RoleClient};

use crate::catalogue::{LiveCatalogue, qualify_prompts, qualify_resources, qualify_tools};

/// A [`ClientHandler`] that re-lists a server's tools when it announces a tool-list change.
#[derive(Clone)]
pub(crate) struct NotifyHandler {
    server: String,
    catalogue: LiveCatalogue,
    timeout: Duration,
    roots: Arc<Vec<rmcp::model::Root>>,
}

impl NotifyHandler {
    /// Build a handler for `server` that updates `catalogue` on list-changed notifications.
    pub(crate) fn new(
        server: String,
        catalogue: LiveCatalogue,
        timeout: Duration,
        roots: Vec<rmcp::model::Root>,
    ) -> Self {
        Self {
            server,
            catalogue,
            timeout,
            roots: Arc::new(roots),
        }
    }
}

impl ClientHandler for NotifyHandler {
    async fn list_roots(
        &self,
        _context: rmcp::service::RequestContext<RoleClient>,
    ) -> Result<ListRootsResult, rmcp::model::ErrorData> {
        Ok(ListRootsResult::new((*self.roots).clone()))
    }

    fn get_info(&self) -> ClientInfo {
        let mut info = ClientInfo::default();
        info.capabilities = ClientCapabilities::builder()
            .enable_roots()
            .enable_roots_list_changed()
            .build();
        info
    }

    async fn on_logging_message(
        &self,
        params: LoggingMessageNotificationParam,
        _context: NotificationContext<RoleClient>,
    ) {
        let redacted_data = redacted_json(&params.data);
        let redacted_for_log = redaction::redact(&redacted_data.to_string());
        tracing::info!(
            target: "cortex::events",
            server = %self.server,
            level = %logging_level_name(params.level),
            logger = params.logger.as_deref().unwrap_or_default(),
            data = %redacted_for_log,
            "received MCP logging notification"
        );
        self.catalogue.publish_log_message(
            &self.server,
            logging_level_name(params.level),
            params.logger,
            redacted_data,
        );
    }

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

fn logging_level_name(level: LoggingLevel) -> &'static str {
    match level {
        LoggingLevel::Debug => "debug",
        LoggingLevel::Info => "info",
        LoggingLevel::Notice => "notice",
        LoggingLevel::Warning => "warning",
        LoggingLevel::Error => "error",
        LoggingLevel::Critical => "critical",
        LoggingLevel::Alert => "alert",
        LoggingLevel::Emergency => "emergency",
    }
}

fn redacted_json(value: &serde_json::Value) -> serde_json::Value {
    let serialized = match serde_json::to_string(value) {
        Ok(serialized) => serialized,
        Err(_) => return serde_json::Value::String(redaction::PLACEHOLDER.to_owned()),
    };
    let redacted = redaction::redact(&serialized);
    serde_json::from_str(&redacted).unwrap_or_else(|_| serde_json::Value::String(redacted))
}
