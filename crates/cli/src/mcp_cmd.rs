//! The `cortex mcp` command surface: inspect the configured MCP servers.
//!
//! `cortex mcp list` loads the layered config, connects every enabled `mcp.<name>` server (spawning
//! its stdio child and completing the MCP handshake), issues `tools/list`, and reports each server's
//! connection status, live PID, and advertised tools under their server-qualified ids. Every spawned
//! child is terminated by PID before the command returns, so the inspection leaves no orphan.

use std::collections::BTreeMap;
use std::io::{self, Write};
use std::time::Duration;

use errors::Exit;
use redaction::guard_outbound;
use serde::Serialize;
use settings::{ConfigInputs, McpServerPersist, McpServerSpec, McpServerSpecTransport};

use crate::cli::{McpAddArgs, McpArgs, McpAuthArgs, McpCommand, McpListArgs, OutputFormat};

/// Dispatch the `cortex mcp` subcommands.
pub fn run(args: McpArgs) -> Exit {
    match args.command {
        McpCommand::List(list) => list_servers(list),
        McpCommand::Add(add) => add_server(add),
        McpCommand::Auth(auth) => auth_server(auth),
    }
}

/// The single JSON document emitted by `cortex mcp add --format json`.
#[derive(Serialize)]
struct AddDocument<'a> {
    r#type: &'a str,
    name: &'a str,
    status: &'a str,
    path: String,
}

/// The single JSON document emitted by `cortex mcp auth --format json`.
#[derive(Serialize)]
struct AuthDocument<'a> {
    r#type: &'a str,
    name: &'a str,
    status: &'a str,
    authorize_url: &'a str,
    redirect_uri: &'a str,
    store: String,
}

const MCP_LIST_OUTBOUND_OPERATION: &str = "MCP remote discovery/connect";
const MCP_AUTH_OUTBOUND_OPERATION: &str = "MCP OAuth discovery/token exchange";

/// Register a new MCP server in the global config file and report the result.
///
/// The definition is persisted under `[mcp.<name>]`. Re-adding the identical definition is
/// idempotent (status `unchanged`); adding a different definition under an existing name errors with
/// a clear message (unless `--force`) and leaves the existing config untouched, so a duplicate add
/// never corrupts the file.
fn add_server(args: McpAddArgs) -> Exit {
    let environment = match parse_key_value_assignments(&args.env, "--env") {
        Ok(environment) => environment,
        Err(message) => {
            eprintln!("cortex mcp add: {message}");
            return Exit::Usage;
        }
    };
    let headers = match parse_key_value_assignments(&args.headers, "--header") {
        Ok(headers) => headers,
        Err(message) => {
            eprintln!("cortex mcp add: {message}");
            return Exit::Usage;
        }
    };
    let transport = if args.url.is_some() {
        if !args.command.is_empty() || args.cwd.is_some() || !environment.is_empty() {
            eprintln!(
                "cortex mcp add: --url registers a remote server and cannot be combined with \
                 --command, --cwd, or --env"
            );
            return Exit::Usage;
        }
        McpServerSpecTransport::Remote
    } else {
        if args.command.is_empty() {
            eprintln!("cortex mcp add: pass either --command <CMD>... or --url <URL>");
            return Exit::Usage;
        }
        if !headers.is_empty() {
            eprintln!("cortex mcp add: --header requires --url for a remote server");
            return Exit::Usage;
        }
        McpServerSpecTransport::Local
    };

    let spec = McpServerSpec {
        transport,
        command: args.command,
        url: args.url,
        headers,
        cwd: args.cwd,
        environment,
        timeout: args.timeout_secs,
        enabled: !args.disabled,
    };

    let inputs = match ConfigInputs::from_process(BTreeMap::new()) {
        Ok(inputs) => inputs,
        Err(err) => {
            eprintln!("cortex mcp add: cannot determine the working directory: {err}");
            return Exit::Failure;
        }
    };

    let outcome = match settings::persist_global_mcp_server(&inputs, &args.name, &spec, args.force)
    {
        Ok(outcome) => outcome,
        Err(err) => {
            eprintln!("cortex mcp add: {err}");
            return Exit::Config;
        }
    };

    let (status, path) = match outcome {
        McpServerPersist::Added(path) => ("added", path),
        McpServerPersist::Unchanged(path) => ("unchanged", path),
        McpServerPersist::Conflict(path) => {
            eprintln!(
                "cortex mcp add: an MCP server named '{}' already exists in {} with a different \
                 definition; pass --force to overwrite it",
                args.name,
                path.display()
            );
            return Exit::Failure;
        }
    };

    emit_add(
        &args.name,
        status,
        path.display().to_string(),
        args.format.format,
    )
}

/// Render the `add` result on stdout in the requested format.
fn emit_add(name: &str, status: &str, path: String, format: OutputFormat) -> Exit {
    let mut body = match format {
        OutputFormat::Text => match status {
            "unchanged" => format!("MCP server '{name}' is already present in {path} (unchanged)"),
            _ => format!("added MCP server '{name}' to {path}"),
        },
        OutputFormat::Json | OutputFormat::Jsonl => {
            let document = AddDocument {
                r#type: "mcp-add",
                name,
                status,
                path,
            };
            serde_json::to_string(&document).expect("mcp-add output is always serializable")
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

/// Run the OAuth authorization-code flow for a configured remote server and persist the resulting
/// token to the process-resolved 0600 MCP credential store.
fn auth_server(args: McpAuthArgs) -> Exit {
    if !crate::feature_gate::network_enabled() {
        return crate::feature_gate::network_unavailable("mcp auth");
    }

    let inputs = match ConfigInputs::from_process(BTreeMap::new()) {
        Ok(inputs) => inputs,
        Err(err) => {
            eprintln!("cortex mcp auth: cannot determine the working directory: {err}");
            return Exit::Failure;
        }
    };
    let effective = match settings::load(inputs) {
        Ok(effective) => effective,
        Err(err) => {
            eprintln!("cortex mcp auth: {err}");
            return Exit::Config;
        }
    };
    let servers = crate::run::mcp_servers_from_config(&effective);
    let Some(server) = servers.get(&args.name) else {
        eprintln!(
            "cortex mcp auth: no MCP server named '{}' is configured",
            args.name
        );
        return Exit::Failure;
    };
    if server.transport != mcp::McpTransport::Remote {
        eprintln!(
            "cortex mcp auth: MCP server '{}' is not remote; OAuth applies only to remote servers",
            args.name
        );
        return Exit::Usage;
    }
    let Some(url) = server.url.as_deref().filter(|url| !url.is_empty()) else {
        eprintln!("cortex mcp auth: remote server '{}' has no url", args.name);
        return Exit::Config;
    };
    let Some(credentials) = crate::mcp_store::store_from_process() else {
        eprintln!(
            "cortex mcp auth: cannot determine an MCP credential-store location; set HOME, \
             XDG_DATA_HOME, or CORTEX_DATA_DIR"
        );
        return Exit::Failure;
    };
    if let Err(diag) = guard_outbound(MCP_AUTH_OUTBOUND_OPERATION) {
        eprintln!("cortex mcp auth: {diag}");
        return diag.exit();
    }

    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(err) => {
            eprintln!("cortex mcp auth: failed to start the async runtime: {err}");
            return Exit::Failure;
        }
    };

    runtime.block_on(async {
        let oauth_config = match mcp::oauth::discover(url, "cortex").await {
            Ok(config) => config,
            Err(err) => {
                eprintln!("cortex mcp auth: {err}");
                return Exit::Failure;
            }
        };
        let request = match mcp::oauth::begin(oauth_config, args.callback_port).await {
            Ok(request) => request,
            Err(err) => {
                eprintln!("cortex mcp auth: {err}");
                return Exit::Failure;
            }
        };
        let authorize_url = request.authorize_url.clone();
        let redirect_uri = request.redirect_uri.clone();
        let store_path = credentials.path().display().to_string();
        if emit_auth_start(
            &args.name,
            &authorize_url,
            &redirect_uri,
            store_path.clone(),
            args.format.format,
        ) != Exit::Success
        {
            return Exit::Failure;
        }

        let token = match request
            .complete(Duration::from_secs(args.timeout_secs))
            .await
        {
            Ok(token) => token,
            Err(err) => {
                eprintln!("cortex mcp auth: {err}");
                return Exit::Failure;
            }
        };
        if let Err(err) = credentials.put(&args.name, token) {
            eprintln!("cortex mcp auth: failed to persist token: {err}");
            return Exit::Failure;
        }
        if args.format.format == OutputFormat::Text {
            println!(
                "MCP server '{}' authorized; token stored in {store_path}",
                args.name
            );
        }
        Exit::Success
    })
}

/// Emit the authorization URL before waiting for the callback, so a browser or validator can visit
/// it while the loopback listener is live. The URL carries only the CSRF state, never a token.
fn emit_auth_start(
    name: &str,
    authorize_url: &str,
    redirect_uri: &str,
    store: String,
    format: OutputFormat,
) -> Exit {
    let mut body = match format {
        OutputFormat::Text => format!(
            "Open this URL to authorize MCP server '{name}':\n{authorize_url}\ncallback: {redirect_uri}\nwaiting for OAuth callback..."
        ),
        OutputFormat::Json | OutputFormat::Jsonl => {
            let document = AuthDocument {
                r#type: "mcp-auth",
                name,
                status: "waiting",
                authorize_url,
                redirect_uri,
                store,
            };
            serde_json::to_string(&document).expect("mcp-auth output is always serializable")
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

/// Parse repeatable `KEY=VALUE` assignments into ordered pairs.
///
/// Each entry must contain `=`; the key (before the first `=`) must be non-empty. The value (after
/// the first `=`) may be empty and may itself contain `=`.
fn parse_key_value_assignments(
    raw: &[String],
    flag: &str,
) -> Result<Vec<(String, String)>, String> {
    let mut pairs = Vec::with_capacity(raw.len());
    for entry in raw {
        let (key, value) = entry
            .split_once('=')
            .ok_or_else(|| format!("invalid {flag} '{entry}': expected KEY=VALUE"))?;
        if key.is_empty() {
            return Err(format!("invalid {flag} '{entry}': the key is empty"));
        }
        pairs.push((key.to_owned(), value.to_owned()));
    }
    Ok(pairs)
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
    resources: Vec<ResourceDocument>,
    prompts: Vec<PromptDocument>,
}

/// One server-qualified tool in the machine-readable listing.
#[derive(Serialize)]
struct ToolDocument {
    id: String,
    tool: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
}

/// One MCP resource in the machine-readable listing.
#[derive(Serialize)]
struct ResourceDocument {
    id: String,
    uri: String,
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    mime_type: Option<String>,
}

/// One MCP prompt in the machine-readable listing.
#[derive(Serialize)]
struct PromptDocument {
    id: String,
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    arguments: Vec<PromptArgumentDocument>,
}

/// One MCP prompt argument in the machine-readable listing.
#[derive(Serialize)]
struct PromptArgumentDocument {
    name: String,
    required: bool,
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
    if !crate::feature_gate::network_enabled() {
        return crate::feature_gate::network_unavailable("mcp list");
    }

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
    if servers
        .values()
        .any(|server| server.enabled && server.transport == mcp::McpTransport::Remote)
        && let Err(diag) = guard_outbound(MCP_LIST_OUTBOUND_OPERATION)
    {
        eprintln!("cortex mcp list: {diag}");
        return diag.exit();
    }

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
        let options = mcp::McpConnectOptions {
            bus: None,
            credentials: crate::mcp_store::store_from_process(),
        };
        let manager = mcp::McpManager::connect_with(servers, options).await;
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
                resources: status
                    .resources
                    .iter()
                    .map(|resource| ResourceDocument {
                        id: resource.id.clone(),
                        uri: resource.uri.clone(),
                        name: resource.name.clone(),
                        description: resource.description.clone(),
                        mime_type: resource.mime_type.clone(),
                    })
                    .collect(),
                prompts: status
                    .prompts
                    .iter()
                    .map(|prompt| PromptDocument {
                        id: prompt.id.clone(),
                        name: prompt.name.clone(),
                        description: prompt.description.clone(),
                        arguments: prompt
                            .arguments
                            .iter()
                            .map(|argument| PromptArgumentDocument {
                                name: argument.name.clone(),
                                required: argument.required,
                                description: argument.description.clone(),
                            })
                            .collect(),
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
        if !server.resources.is_empty() {
            let uris = server
                .resources
                .iter()
                .map(|resource| resource.uri.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            line.push_str(&format!("  resources: {uris}"));
        }
        if !server.prompts.is_empty() {
            let names = server
                .prompts
                .iter()
                .map(|prompt| prompt.name.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            line.push_str(&format!("  prompts: {names}"));
        }
        lines.push(line);
    }
    lines.join("\n")
}
