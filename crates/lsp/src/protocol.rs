//! Minimal hand-rolled Language Server Protocol lifecycle types.

use serde::Serialize;
use serde_json::{Map, Value, json};

use crate::LspError;

/// The JSON-RPC protocol version used on the wire.
pub const JSONRPC_VERSION: &str = "2.0";

/// Identifies the Cortex Code client to the server.
#[derive(Debug, Clone, Serialize)]
pub struct ClientInfo {
    /// Human-readable client name.
    pub name: String,
    /// Optional client version.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
}

/// Parameters sent with the `initialize` request.
#[derive(Debug, Clone)]
pub struct InitializeParams {
    /// The OS process id of the client, or `None`.
    pub process_id: Option<u32>,
    /// The workspace root URI.
    pub root_uri: Option<String>,
    /// Optional client identification.
    pub client_info: Option<ClientInfo>,
    /// Client capabilities advertised to the server.
    pub capabilities: Value,
}

impl Default for InitializeParams {
    fn default() -> Self {
        Self {
            process_id: Some(std::process::id()),
            root_uri: None,
            client_info: Some(ClientInfo {
                name: "cortex-code".to_owned(),
                version: Some(env!("CARGO_PKG_VERSION").to_owned()),
            }),
            capabilities: json!({
                "textDocument": {
                    "publishDiagnostics": {
                        "relatedInformation": true
                    }
                },
                "workspace": {
                    "diagnostics": {
                        "refreshSupport": true
                    }
                }
            }),
        }
    }
}

impl InitializeParams {
    /// A sensible default set of initialize parameters.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the workspace root URI.
    #[must_use]
    pub fn with_root_uri(mut self, root_uri: impl Into<String>) -> Self {
        self.root_uri = Some(root_uri.into());
        self
    }

    /// Override the client process id.
    #[must_use]
    pub const fn with_process_id(mut self, process_id: Option<u32>) -> Self {
        self.process_id = process_id;
        self
    }

    /// Serialize to JSON-RPC `params`.
    pub(crate) fn to_value(&self) -> Value {
        let mut map = Map::new();
        map.insert(
            "processId".to_owned(),
            self.process_id.map_or(Value::Null, Value::from),
        );
        map.insert(
            "rootUri".to_owned(),
            self.root_uri
                .as_ref()
                .map_or(Value::Null, |uri| Value::from(uri.clone())),
        );
        if let Some(info) = &self.client_info {
            if let Ok(value) = serde_json::to_value(info) {
                map.insert("clientInfo".to_owned(), value);
            }
        }
        map.insert("capabilities".to_owned(), self.capabilities.clone());
        Value::Object(map)
    }
}

/// Identifies the server returned from `initialize`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerInfo {
    name: String,
    version: Option<String>,
}

impl ServerInfo {
    /// The server-reported name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The server-reported version.
    #[must_use]
    pub fn version(&self) -> Option<&str> {
        self.version.as_deref()
    }
}

/// The raw server capabilities object with small convenience helpers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerCapabilities {
    raw: Value,
}

impl ServerCapabilities {
    pub(crate) const fn new(raw: Value) -> Self {
        Self { raw }
    }

    /// The raw capabilities JSON value.
    #[must_use]
    pub const fn raw(&self) -> &Value {
        &self.raw
    }
}

/// The parsed result of a successful `initialize` request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InitializeResult {
    capabilities: ServerCapabilities,
    server_info: Option<ServerInfo>,
}

impl InitializeResult {
    /// The server capabilities.
    #[must_use]
    pub const fn capabilities(&self) -> &ServerCapabilities {
        &self.capabilities
    }

    /// The server info, if provided.
    #[must_use]
    pub const fn server_info(&self) -> Option<&ServerInfo> {
        self.server_info.as_ref()
    }

    pub(crate) fn from_result_value(result: Value) -> Result<Self, LspError> {
        let capabilities = result
            .get("capabilities")
            .cloned()
            .ok_or(LspError::Protocol)?;
        let server_info = result.get("serverInfo").and_then(parse_server_info);
        Ok(Self {
            capabilities: ServerCapabilities::new(capabilities),
            server_info,
        })
    }
}

fn parse_server_info(value: &Value) -> Option<ServerInfo> {
    Some(ServerInfo {
        name: value.get("name")?.as_str()?.to_owned(),
        version: value
            .get("version")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initialize_params_include_diagnostics_capabilities() {
        let value = InitializeParams::new()
            .with_process_id(Some(7))
            .with_root_uri("file:///work")
            .to_value();
        assert_eq!(value.get("processId"), Some(&json!(7)));
        assert_eq!(value.get("rootUri"), Some(&json!("file:///work")));
        assert!(value.pointer("/textDocument/publishDiagnostics").is_none());
        assert!(
            value
                .pointer("/capabilities/textDocument/publishDiagnostics")
                .is_some()
        );
    }

    #[test]
    fn initialize_result_requires_capabilities() {
        assert_eq!(
            InitializeResult::from_result_value(json!({"serverInfo":{"name":"x"}})),
            Err(LspError::Protocol)
        );
    }
}
