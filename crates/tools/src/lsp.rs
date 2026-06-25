//! The `lsp` navigation tool: hover, definition, references, and symbols from a real language
//! server attached by the surface.

use futures_util::future::BoxFuture;
use serde_json::{Value, json};

use crate::tool::{LspOperation, LspRequest, Tool, ToolContext, ToolOutput};

const DESCRIPTION: &str = "Query the project's real Language Server Protocol server. Use this \
to get hover type/signature/docs, go-to-definition locations, references, implementation \
locations, call hierarchy, document symbols, or workspace symbols. Positions are 1-based line and \
1-based character offsets over the target symbol. For workspaceSymbols, include filePath when \
possible so Cortex can select the matching language server.";

/// The `lsp` tool.
pub struct LspTool;

impl Tool for LspTool {
    fn name(&self) -> &str {
        "lsp"
    }

    fn description(&self) -> &str {
        DESCRIPTION
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": [
                        "hover",
                        "definition",
                        "references",
                        "implementation",
                        "prepareCallHierarchy",
                        "incomingCalls",
                        "outgoingCalls",
                        "documentSymbols",
                        "workspaceSymbols"
                    ],
                    "description": "The LSP operation to run."
                },
                "filePath": {
                    "type": "string",
                    "description": "The file to query (absolute or relative to the working directory). Required except for workspaceSymbols, where it is an optional server-selection hint."
                },
                "line": {
                    "type": "integer",
                    "minimum": 1,
                    "description": "1-based line number over the target symbol (required for hover, definition, references, implementation, and call hierarchy actions)."
                },
                "character": {
                    "type": "integer",
                    "minimum": 1,
                    "description": "1-based character offset over the target symbol (required for hover, definition, references, implementation, and call hierarchy actions)."
                },
                "query": {
                    "type": "string",
                    "description": "Workspace symbol query (required for workspaceSymbols)."
                },
                "includeDeclaration": {
                    "type": "boolean",
                    "description": "For references, include the declaration location (default true)."
                }
            },
            "required": ["action"]
        })
    }

    fn execute<'a>(&'a self, arguments: Value, ctx: &'a ToolContext) -> BoxFuture<'a, ToolOutput> {
        Box::pin(async move { run(&arguments, ctx) })
    }
}

fn run(arguments: &Value, ctx: &ToolContext) -> ToolOutput {
    let request = match parse_request(arguments) {
        Ok(request) => request,
        Err(message) => return ToolOutput::error(message),
    };

    let permission_target = request
        .file_path
        .as_deref()
        .or(request.query.as_deref())
        .unwrap_or("workspace");
    if let Err(message) = ctx.ensure_permitted("lsp", permission_target) {
        return ToolOutput::error(message);
    }

    let Some(service) = ctx.lsp_service() else {
        return ToolOutput::text(
            "lsp: no server available because this surface did not attach an LSP service",
        );
    };
    service.query(request)
}

fn parse_request(arguments: &Value) -> Result<LspRequest, String> {
    let action = string_arg(arguments, "action")
        .or_else(|| string_arg(arguments, "operation"))
        .ok_or_else(|| "lsp: the 'action' argument is required".to_owned())?;
    let operation = parse_operation(&action)?;
    let file_path = string_arg(arguments, "filePath")
        .or_else(|| string_arg(arguments, "file_path"))
        .or_else(|| string_arg(arguments, "path"));
    let line = u32_arg(arguments, "line");
    let character = u32_arg(arguments, "character")
        .or_else(|| u32_arg(arguments, "column"))
        .or_else(|| u32_arg(arguments, "col"));
    let query = string_arg(arguments, "query");
    let include_declaration = bool_arg(arguments, "includeDeclaration")
        .or_else(|| bool_arg(arguments, "include_declaration"))
        .unwrap_or(true);

    match operation {
        LspOperation::Hover
        | LspOperation::Definition
        | LspOperation::References
        | LspOperation::Implementation
        | LspOperation::PrepareCallHierarchy
        | LspOperation::IncomingCalls
        | LspOperation::OutgoingCalls => {
            require_file(&file_path)?;
            require_positive(line, "line")?;
            require_positive(character, "character")?;
        }
        LspOperation::DocumentSymbols => {
            require_file(&file_path)?;
        }
        LspOperation::WorkspaceSymbols => {
            if query.as_deref().is_none_or(str::is_empty) {
                return Err("lsp: the 'query' argument is required for workspaceSymbols".to_owned());
            }
        }
    }

    Ok(LspRequest {
        operation,
        file_path,
        line,
        character,
        query,
        include_declaration,
    })
}

fn parse_operation(action: &str) -> Result<LspOperation, String> {
    match action.trim() {
        "hover" => Ok(LspOperation::Hover),
        "definition" | "goToDefinition" | "gotoDefinition" => Ok(LspOperation::Definition),
        "references" => Ok(LspOperation::References),
        "implementation" | "goToImplementation" | "gotoImplementation" => {
            Ok(LspOperation::Implementation)
        }
        "prepareCallHierarchy" | "prepare_call_hierarchy" | "prepare-call-hierarchy" => {
            Ok(LspOperation::PrepareCallHierarchy)
        }
        "incomingCalls" | "incoming_calls" | "incoming-calls" => Ok(LspOperation::IncomingCalls),
        "outgoingCalls" | "outgoing_calls" | "outgoing-calls" => Ok(LspOperation::OutgoingCalls),
        "documentSymbols" | "document_symbols" | "document-symbols" => {
            Ok(LspOperation::DocumentSymbols)
        }
        "workspaceSymbols" | "workspace_symbols" | "workspace-symbols" => {
            Ok(LspOperation::WorkspaceSymbols)
        }
        other => Err(format!(
            "lsp: unknown action '{other}' (expected hover, definition, references, implementation, prepareCallHierarchy, incomingCalls, outgoingCalls, documentSymbols, or workspaceSymbols)"
        )),
    }
}

fn require_file(file_path: &Option<String>) -> Result<(), String> {
    if file_path.as_deref().is_none_or(str::is_empty) {
        Err("lsp: the 'filePath' argument is required for this action".to_owned())
    } else {
        Ok(())
    }
}

fn require_positive(value: Option<u32>, name: &str) -> Result<(), String> {
    match value {
        Some(value) if value > 0 => Ok(()),
        _ => Err(format!(
            "lsp: the '{name}' argument must be a positive 1-based integer"
        )),
    }
}

fn string_arg(arguments: &Value, name: &str) -> Option<String> {
    arguments
        .get(name)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn u32_arg(arguments: &Value, name: &str) -> Option<u32> {
    arguments
        .get(name)
        .and_then(Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
}

fn bool_arg(arguments: &Value, name: &str) -> Option<bool> {
    arguments.get(name).and_then(Value::as_bool)
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::tool::LspService;

    #[derive(Default)]
    struct RecordingLsp {
        requests: Mutex<Vec<LspRequest>>,
    }

    impl LspService for RecordingLsp {
        fn query(&self, request: LspRequest) -> ToolOutput {
            self.requests.lock().unwrap().push(request);
            ToolOutput::text("lsp ok")
        }
    }

    #[tokio::test]
    async fn parses_position_request_and_calls_service() {
        let service = Arc::new(RecordingLsp::default());
        let ctx = ToolContext::new(std::env::temp_dir()).with_lsp_service(service.clone());
        let output = LspTool
            .execute(
                json!({"action":"hover","filePath":"src/lib.rs","line":2,"character":8}),
                &ctx,
            )
            .await;

        assert!(!output.is_error, "{}", output.model_content());
        let requests = service.requests.lock().unwrap();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].operation, LspOperation::Hover);
        assert_eq!(requests[0].file_path.as_deref(), Some("src/lib.rs"));
        assert_eq!(requests[0].line, Some(2));
        assert_eq!(requests[0].character, Some(8));
    }

    #[tokio::test]
    async fn missing_service_is_a_clean_no_server_result() {
        let ctx = ToolContext::new(std::env::temp_dir());
        let output = LspTool
            .execute(
                json!({"action":"documentSymbols","filePath":"README.md"}),
                &ctx,
            )
            .await;

        assert!(!output.is_error, "{}", output.model_content());
        assert!(output.model_content().contains("no server available"));
    }

    #[test]
    fn parse_request_accepts_implementation_and_call_hierarchy_actions() {
        let implementation = parse_request(
            &json!({"action":"goToImplementation","filePath":"src/lib.rs","line":5,"character":3}),
        )
        .expect("implementation request should parse");
        assert_eq!(implementation.operation, LspOperation::Implementation);

        let prepare = parse_request(&json!({
            "action":"prepareCallHierarchy",
            "filePath":"src/lib.rs",
            "line":5,
            "character":3
        }))
        .expect("prepare request should parse");
        assert_eq!(prepare.operation, LspOperation::PrepareCallHierarchy);

        let incoming = parse_request(&json!({
            "action":"incoming_calls",
            "filePath":"src/lib.rs",
            "line":5,
            "character":3
        }))
        .expect("incoming request should parse");
        assert_eq!(incoming.operation, LspOperation::IncomingCalls);

        let outgoing = parse_request(&json!({
            "action":"outgoing-calls",
            "filePath":"src/lib.rs",
            "line":5,
            "character":3
        }))
        .expect("outgoing request should parse");
        assert_eq!(outgoing.operation, LspOperation::OutgoingCalls);
    }

    #[test]
    fn parse_request_keeps_reference_include_declaration_and_action_validation() {
        let references = parse_request(&json!({
            "action":"references",
            "filePath":"src/lib.rs",
            "line":5,
            "character":3,
            "includeDeclaration": false
        }))
        .expect("references request should parse");
        assert_eq!(references.operation, LspOperation::References);
        assert!(!references.include_declaration);

        let workspace = parse_request(&json!({
            "action":"workspaceSymbols",
            "query":"add_one"
        }))
        .expect("workspace symbols should not require filePath");
        assert_eq!(workspace.operation, LspOperation::WorkspaceSymbols);
        assert!(workspace.file_path.is_none());

        let err = parse_request(&json!({
            "action":"incomingCalls",
            "filePath":"src/lib.rs",
            "character":3
        }))
        .expect_err("incoming calls should require line and character");
        assert!(err.contains("line"));
    }
}
