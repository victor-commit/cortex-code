//! The `cortex mcp` command surface: inspect the configured MCP servers.
//!
//! `cortex mcp list` loads the layered config, connects every enabled `mcp.<name>` server (spawning
//! its stdio child and completing the MCP handshake), issues `tools/list`, and reports each server's
//! connection status, live PID, and advertised tools under their server-qualified ids. Every spawned
//! child is terminated by PID before the command returns, so the inspection leaves no orphan.

use std::collections::BTreeMap;
use std::io::{self, Write};

use errors::Exit;
use serde::Serialize;
use settings::ConfigInputs;

use crate::cli::{McpArgs, McpCommand, McpListArgs, OutputFormat};

/// Dispatch the `cortex mcp` subcommands.
pub fn run(args: McpArgs) -> Exit {
    match args.command {
        McpCommand::List(list) => list_servers(list),
    }
}

/// One server's reported status in the machine-readable listing.
#[derive(Serialize)]
struct ServerDocument {
    name: String,
    status: &'static str,
    pid: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
    tools: Vec<ToolDocument>,
}

/// One server-qualified tool in the machine-readable listing.
#[derive(Serialize)]
struct ToolDocument {
    id: String,
    tool: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
}

/// The single JSON document emitted by `cortex mcp list --format json`.
#[derive(Serialize)]
struct ListDocument {
    r#type: &'static str,
    servers: Vec<ServerDocument>,
}

/// Connect every configured server, report its status and tools, then shut them all down.
fn list_servers(args: McpListArgs) -> Exit {
    let inputs = match ConfigInputs::from_process(BTreeMap::new()) {
        Ok(inputs) => inputs,
        Err(err) => {
            eprintln!("cortex mcp list: cannot determine the working directory: {err}");
            return Exit::Failure;
        }
    };
    let effective = match settings::load(inputs) {
        Ok(effective) => effective,
        Err(err) => {
            eprintln!("cortex mcp list: {err}");
            return Exit::Config;
        }
    };

    let servers = crate::run::mcp_servers_from_config(&effective);

    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(err) => {
            eprintln!("cortex mcp list: failed to start the async runtime: {err}");
            return Exit::Failure;
        }
    };

    let documents = runtime.block_on(async {
        let manager = mcp::McpManager::connect(servers).await;
        let documents = manager
            .statuses()
            .iter()
            .map(|status| ServerDocument {
                name: status.name.clone(),
                status: status.state.as_str(),
                pid: status.pid,
                error: status.error.clone(),
                tools: status
                    .tools
                    .iter()
                    .map(|tool| ToolDocument {
                        id: tool.id.clone(),
                        tool: tool.tool.clone(),
                        description: tool.description.clone(),
                    })
                    .collect(),
            })
            .collect::<Vec<_>>();
        manager.shutdown().await;
        documents
    });

    emit(documents, args.format.format)
}

/// Render the listing on stdout in the requested format.
fn emit(servers: Vec<ServerDocument>, format: OutputFormat) -> Exit {
    let mut body = match format {
        OutputFormat::Text => render_text(&servers),
        OutputFormat::Json | OutputFormat::Jsonl => {
            let document = ListDocument {
                r#type: "mcp-list",
                servers,
            };
            serde_json::to_string(&document).expect("mcp-list output is always serializable")
        }
    };
    body.push('\n');

    let stdout = io::stdout();
    let mut handle = stdout.lock();
    if handle
        .write_all(body.as_bytes())
        .and_then(|()| handle.flush())
        .is_err()
    {
        return Exit::Failure;
    }
    Exit::Success
}

/// Render the human-readable listing.
fn render_text(servers: &[ServerDocument]) -> String {
    if servers.is_empty() {
        return "no MCP servers configured".to_owned();
    }
    let mut lines = Vec::with_capacity(servers.len());
    for server in servers {
        let mut line = format!("{}  {}", server.name, server.status);
        if let Some(pid) = server.pid {
            line.push_str(&format!("  pid={pid}"));
        }
        if let Some(error) = &server.error {
            line.push_str(&format!("  ({error})"));
        }
        if !server.tools.is_empty() {
            let ids = server
                .tools
                .iter()
                .map(|tool| tool.id.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            line.push_str(&format!("  tools: {ids}"));
        }
        lines.push(line);
    }
    lines.join("\n")
}
