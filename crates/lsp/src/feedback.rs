//! Post-mutation diagnostics feedback built on real language servers.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use redaction::guard_outbound;
use url::Url;

use crate::client::LspClient;
use crate::config::{InstallPlan, LspConfigSet, LspServerConfig};
use crate::diagnostics::{DiagnosticSeverity, Position};
use crate::navigation::{DocumentSymbol, Hover, Location, SymbolInformation};
use crate::protocol::InitializeParams;

/// Total wall-clock budget for one post-mutation LSP diagnostic collection.
pub const FEEDBACK_BUDGET: Duration = Duration::from_millis(4500);
/// Maximum diagnostics rendered into a tool result.
pub const MAX_RENDERED_DIAGNOSTICS: usize = 20;
/// Maximum characters from one diagnostic message.
pub const MAX_MESSAGE_CHARS: usize = 500;
const NAVIGATION_RETRY_INTERVAL: Duration = Duration::from_millis(150);
// LSP cancellation/readiness errors that rust-analyzer may return while a just-opened workspace is
// still loading or while it invalidates an in-flight query after didOpen/didChange. Treat these as
// retryable for navigation only, without weakening fatal transport/process handling.
const LSP_REQUEST_CANCELLED: i64 = -32800;
const LSP_CONTENT_MODIFIED: i64 = -32801;
const LSP_SERVER_CANCELLED: i64 = -32802;

/// One diagnostic projected to tool-result shape.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileDiagnostic {
    /// Stable lowercase severity label (`error`, `warning`, `information`, `hint`, or `note`).
    pub severity: &'static str,
    /// One-based line number for compatibility with existing tool result prose.
    pub line: u32,
    /// One-based start character on the line.
    pub character: u32,
    /// One-based end line.
    pub end_line: u32,
    /// One-based end character.
    pub end_character: u32,
    /// Optional diagnostic source.
    pub source: Option<String>,
    /// Optional diagnostic code.
    pub code: Option<String>,
    /// Human-readable diagnostic message.
    pub message: String,
}

/// Result of one best-effort diagnostic collection attempt.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FeedbackOutcome {
    /// No server applies, binary is missing, install was disabled, or startup failed.
    NoServer,
    /// A previously running server died or stopped responding.
    Unavailable,
    /// Diagnostics currently known for the file. Empty means the file is clean.
    Diagnostics(Vec<FileDiagnostic>),
}

/// A read-only LSP navigation request. Line and character inputs are 1-based because they are
/// model/user-facing; they are converted to zero-based LSP positions on the wire.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum NavigationRequest {
    /// Hover at a symbol position.
    Hover {
        file_path: PathBuf,
        line: u32,
        character: u32,
    },
    /// Go to definition from a symbol use site.
    Definition {
        file_path: PathBuf,
        line: u32,
        character: u32,
    },
    /// Find references to a symbol.
    References {
        file_path: PathBuf,
        line: u32,
        character: u32,
        include_declaration: bool,
    },
    /// List document symbols for a file.
    DocumentSymbols { file_path: PathBuf },
    /// Resolve workspace symbols. Supplying `file_path` selects the language server by extension;
    /// omitting it uses the first available configured server.
    WorkspaceSymbols {
        query: String,
        file_path: Option<PathBuf>,
    },
}

/// Result of a read-only LSP navigation request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NavigationOutcome {
    /// No server applies or no configured server can be launched.
    NoServer,
    /// A running server stopped responding, crashed, or returned a transport error.
    Unavailable,
    /// Hover result.
    Hover(Option<Hover>),
    /// Definition/reference locations.
    Locations(Vec<Location>),
    /// Document symbols.
    DocumentSymbols(Vec<DocumentSymbol>),
    /// Workspace symbols.
    WorkspaceSymbols(Vec<SymbolInformation>),
}

/// A point-in-time view of one configured server.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ServerStatus {
    /// Language id.
    pub language_id: String,
    /// Program name.
    pub program: String,
    /// Current state.
    pub state: ServerRunState,
}

/// Coarse state for one managed server.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ServerRunState {
    /// Not started yet.
    Idle,
    /// Running with OS PID.
    Running(u32),
    /// Missing from PATH and not installable under current policy.
    Missing,
    /// Startup failed.
    Failed,
    /// Was running and then stopped or crashed.
    Stopped,
}

impl ServerRunState {
    /// Stable lowercase label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Running(_) => "running",
            Self::Missing => "missing",
            Self::Failed => "failed",
            Self::Stopped => "stopped",
        }
    }
}

enum ServerSlot {
    Running {
        client: Box<LspClient>,
        open_versions: BTreeMap<String, i64>,
    },
    Missing,
    Failed,
    Broken,
}

/// Lazy per-language server pool plus bounded diagnostics collection.
pub struct FeedbackManager {
    root: PathBuf,
    config: LspConfigSet,
    servers: BTreeMap<String, ServerSlot>,
    budget: Duration,
}

impl FeedbackManager {
    /// Create a manager rooted at `root`.
    #[must_use]
    pub fn new(root: impl Into<PathBuf>, config: LspConfigSet) -> Self {
        Self {
            root: root.into(),
            config,
            servers: BTreeMap::new(),
            budget: FEEDBACK_BUDGET,
        }
    }

    /// Override collection budget.
    #[must_use]
    pub const fn with_budget(mut self, budget: Duration) -> Self {
        self.budget = budget;
        self
    }

    /// Best-effort diagnostics for `path`. Never fails the mutation.
    pub fn collect(&mut self, path: &Path) -> FeedbackOutcome {
        let deadline = Instant::now() + self.budget;
        let absolute = if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.root.join(path)
        };
        let Some(server) = self.config.resolve_for_path(&absolute).cloned() else {
            return FeedbackOutcome::NoServer;
        };
        let language_id = server.language_id().to_owned();

        match self.servers.get(&language_id) {
            Some(ServerSlot::Missing | ServerSlot::Failed) => return FeedbackOutcome::NoServer,
            Some(ServerSlot::Broken) => return FeedbackOutcome::Unavailable,
            Some(ServerSlot::Running { .. }) | None => {}
        }
        if !self.servers.contains_key(&language_id) && !self.start_server(&server, deadline) {
            return match self.servers.get(&language_id) {
                Some(ServerSlot::Missing) => FeedbackOutcome::NoServer,
                Some(ServerSlot::Failed) => FeedbackOutcome::Unavailable,
                Some(ServerSlot::Broken) => FeedbackOutcome::Unavailable,
                Some(ServerSlot::Running { .. }) | None => FeedbackOutcome::NoServer,
            };
        }

        let Ok(text) = std::fs::read_to_string(&absolute) else {
            return FeedbackOutcome::NoServer;
        };
        let uri = path_to_uri(&absolute);
        let Some(ServerSlot::Running { client, .. }) = self.servers.get_mut(&language_id) else {
            return FeedbackOutcome::Unavailable;
        };
        // Drain stale publishes before this mutation establishes its didOpen/didChange boundary.
        // Anything already buffered belongs to an older generation and must not satisfy the fresh
        // post-mutation diagnostics wait below.
        let _ = client.drain_pending_diagnostics();
        let baseline_generation = client.diagnostic_generation();

        let document_version = match self.announce(&language_id, &uri, &text) {
            Ok(version) => version,
            Err(()) => {
                self.mark_broken(&language_id);
                return FeedbackOutcome::Unavailable;
            }
        };

        let remaining = deadline
            .checked_duration_since(Instant::now())
            .unwrap_or_default();
        let Some(ServerSlot::Running { client, .. }) = self.servers.get_mut(&language_id) else {
            return FeedbackOutcome::Unavailable;
        };
        let fresh = match client.poll_fresh_diagnostics(
            &uri,
            document_version,
            baseline_generation,
            remaining,
        ) {
            Ok(fresh) => fresh,
            Err(_) => {
                self.mark_broken(&language_id);
                return FeedbackOutcome::Unavailable;
            }
        };
        let diagnostics = fresh
            .unwrap_or_default()
            .iter()
            .map(|diagnostic| FileDiagnostic {
                severity: diagnostic
                    .severity
                    .map_or("note", DiagnosticSeverity::label),
                line: diagnostic.range.start.line.saturating_add(1),
                character: diagnostic.range.start.character.saturating_add(1),
                end_line: diagnostic.range.end.line.saturating_add(1),
                end_character: diagnostic.range.end.character.saturating_add(1),
                source: diagnostic.source.clone(),
                code: diagnostic.code.clone(),
                message: diagnostic.message.clone(),
            })
            .collect();
        FeedbackOutcome::Diagnostics(diagnostics)
    }

    /// Execute a read-only navigation request against the real language server. A missing or dead
    /// server is surfaced as a typed outcome rather than a panic or hang.
    pub fn navigate(&mut self, request: NavigationRequest) -> NavigationOutcome {
        let deadline = Instant::now() + self.budget;
        match request {
            NavigationRequest::Hover {
                file_path,
                line,
                character,
            } => self.with_document(&file_path, deadline, |client, uri| {
                let position = one_based_to_lsp(line, character);
                retry_navigation(deadline, || client.hover(uri, position), Option::is_some)
            }),
            NavigationRequest::Definition {
                file_path,
                line,
                character,
            } => self.with_document(&file_path, deadline, |client, uri| {
                let position = one_based_to_lsp(line, character);
                retry_navigation(
                    deadline,
                    || client.goto_definition(uri, position),
                    |locations| !locations.is_empty(),
                )
                .map(NavigationOutcome::Locations)
            }),
            NavigationRequest::References {
                file_path,
                line,
                character,
                include_declaration,
            } => self.with_document(&file_path, deadline, |client, uri| {
                let position = one_based_to_lsp(line, character);
                retry_navigation(
                    deadline,
                    || client.references(uri, position, include_declaration),
                    |locations| !locations.is_empty(),
                )
                .map(NavigationOutcome::Locations)
            }),
            NavigationRequest::DocumentSymbols { file_path } => {
                self.with_document(&file_path, deadline, |client, uri| {
                    retry_navigation(
                        deadline,
                        || client.document_symbols(uri),
                        |symbols| !symbols.is_empty(),
                    )
                    .map(NavigationOutcome::DocumentSymbols)
                })
            }
            NavigationRequest::WorkspaceSymbols { query, file_path } => {
                if let Some(file_path) = file_path {
                    self.with_document(&file_path, deadline, |client, _uri| {
                        retry_navigation(
                            deadline,
                            || client.workspace_symbols(&query),
                            |symbols| !symbols.is_empty(),
                        )
                        .map(NavigationOutcome::WorkspaceSymbols)
                    })
                } else {
                    self.with_any_server(deadline, |client| {
                        retry_navigation(
                            deadline,
                            || client.workspace_symbols(&query),
                            |symbols| !symbols.is_empty(),
                        )
                        .map(NavigationOutcome::WorkspaceSymbols)
                    })
                }
            }
        }
    }

    /// Point-in-time status for every configured server.
    #[must_use]
    pub fn statuses(&mut self) -> Vec<ServerStatus> {
        let mut rows = Vec::new();
        for server in self.config.servers() {
            let state = match self.servers.get_mut(server.language_id()) {
                None => {
                    if let Some(pid) = find_running_pid(server.program()) {
                        ServerRunState::Running(pid)
                    } else if command_exists(server.program()) {
                        ServerRunState::Idle
                    } else {
                        ServerRunState::Missing
                    }
                }
                Some(ServerSlot::Running { client, .. }) => {
                    if client.is_running() {
                        ServerRunState::Running(client.child_id())
                    } else {
                        ServerRunState::Stopped
                    }
                }
                Some(ServerSlot::Missing) => ServerRunState::Missing,
                Some(ServerSlot::Failed) => ServerRunState::Failed,
                Some(ServerSlot::Broken) => ServerRunState::Stopped,
            };
            rows.push(ServerStatus {
                language_id: server.language_id().to_owned(),
                program: server.program().to_owned(),
                state,
            });
        }
        rows
    }

    /// Shut down every running server by its child handle/PID.
    pub fn shutdown_all(&mut self) {
        let languages: Vec<String> = self.servers.keys().cloned().collect();
        for language_id in languages {
            if let Some(ServerSlot::Running { mut client, .. }) = self.servers.remove(&language_id)
            {
                let _ = client.shutdown();
                let _ = client.exit();
                self.servers.insert(language_id, ServerSlot::Broken);
            }
        }
    }

    fn start_server(&mut self, server: &LspServerConfig, deadline: Instant) -> bool {
        match ensure_installed(server) {
            InstallOutcome::Available => {}
            InstallOutcome::Unavailable | InstallOutcome::NetDisabled => {
                self.servers
                    .insert(server.language_id().to_owned(), ServerSlot::Missing);
                return false;
            }
        }

        let remaining = deadline
            .checked_duration_since(Instant::now())
            .unwrap_or_default();
        let command = server
            .to_command()
            .with_request_timeout(remaining.max(Duration::from_millis(1)));
        let root_uri = path_to_uri(&self.root);
        let started = LspClient::spawn(&command).ok().and_then(|mut client| {
            client
                .initialize(&InitializeParams::new().with_root_uri(root_uri))
                .ok()?;
            client.initialized().ok()?;
            Some(client)
        });
        match started {
            Some(client) => {
                self.servers.insert(
                    server.language_id().to_owned(),
                    ServerSlot::Running {
                        client: Box::new(client),
                        open_versions: BTreeMap::new(),
                    },
                );
                true
            }
            None => {
                self.servers
                    .insert(server.language_id().to_owned(), ServerSlot::Failed);
                false
            }
        }
    }

    fn announce(&mut self, language_id: &str, uri: &str, text: &str) -> Result<i64, ()> {
        let Some(ServerSlot::Running {
            client,
            open_versions,
        }) = self.servers.get_mut(language_id)
        else {
            return Err(());
        };
        let version = match open_versions.get(uri).copied() {
            None => {
                let version = 1;
                client.set_minimum_diagnostic_version(uri, version);
                client
                    .did_open(uri, language_id, version, text)
                    .map_err(|_| ())?;
                open_versions.insert(uri.to_owned(), version);
                version
            }
            Some(version) => {
                let next = version.saturating_add(1);
                client.set_minimum_diagnostic_version(uri, next);
                client.did_change_full(uri, next, text).map_err(|_| ())?;
                open_versions.insert(uri.to_owned(), next);
                next
            }
        };
        Ok(version)
    }

    fn mark_broken(&mut self, language_id: &str) {
        self.servers
            .insert(language_id.to_owned(), ServerSlot::Broken);
    }

    fn with_document<T, F>(&mut self, path: &Path, deadline: Instant, op: F) -> NavigationOutcome
    where
        T: Into<NavigationOutcome>,
        F: FnOnce(&mut LspClient, &str) -> Result<T, crate::LspError>,
    {
        let Ok((language_id, uri)) = self.ensure_document(path, deadline) else {
            return match self.config.resolve_for_path(self.absolute_path(path)) {
                Some(server) => match self.servers.get(server.language_id()) {
                    Some(ServerSlot::Broken | ServerSlot::Failed) => NavigationOutcome::Unavailable,
                    _ => NavigationOutcome::NoServer,
                },
                None => NavigationOutcome::NoServer,
            };
        };
        let Some(ServerSlot::Running { client, .. }) = self.servers.get_mut(&language_id) else {
            return NavigationOutcome::Unavailable;
        };
        match op(client, &uri) {
            Ok(outcome) => outcome.into(),
            Err(_) => {
                self.mark_broken(&language_id);
                NavigationOutcome::Unavailable
            }
        }
    }

    fn with_any_server<T, F>(&mut self, deadline: Instant, op: F) -> NavigationOutcome
    where
        T: Into<NavigationOutcome>,
        F: FnOnce(&mut LspClient) -> Result<T, crate::LspError>,
    {
        let Some(language_id) = self.ensure_any_server(deadline) else {
            return NavigationOutcome::NoServer;
        };
        let Some(ServerSlot::Running { client, .. }) = self.servers.get_mut(&language_id) else {
            return NavigationOutcome::Unavailable;
        };
        match op(client) {
            Ok(outcome) => outcome.into(),
            Err(_) => {
                self.mark_broken(&language_id);
                NavigationOutcome::Unavailable
            }
        }
    }

    fn ensure_document(&mut self, path: &Path, deadline: Instant) -> Result<(String, String), ()> {
        let absolute = self.absolute_path(path);
        let Some(server) = self.config.resolve_for_path(&absolute).cloned() else {
            return Err(());
        };
        let language_id = server.language_id().to_owned();
        match self.servers.get(&language_id) {
            Some(ServerSlot::Missing | ServerSlot::Failed | ServerSlot::Broken) => return Err(()),
            Some(ServerSlot::Running { .. }) | None => {}
        }
        if !self.servers.contains_key(&language_id) && !self.start_server(&server, deadline) {
            return Err(());
        }
        let text = std::fs::read_to_string(&absolute).map_err(|_| ())?;
        let uri = path_to_uri(&absolute);
        if self.announce(&language_id, &uri, &text).is_err() {
            self.mark_broken(&language_id);
            return Err(());
        }
        Ok((language_id, uri))
    }

    fn ensure_any_server(&mut self, deadline: Instant) -> Option<String> {
        if let Some((language_id, client)) =
            self.servers
                .iter_mut()
                .find_map(|(language_id, slot)| match slot {
                    ServerSlot::Running { client, .. } => Some((language_id.clone(), client)),
                    ServerSlot::Missing | ServerSlot::Failed | ServerSlot::Broken => None,
                })
        {
            if client.is_running() {
                return Some(language_id);
            }
        }
        let server = self
            .config
            .servers()
            .iter()
            .find(|server| command_exists(server.program()))
            .cloned()?;
        let language_id = server.language_id().to_owned();
        self.start_server(&server, deadline).then_some(language_id)
    }

    fn absolute_path(&self, path: &Path) -> PathBuf {
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.root.join(path)
        }
    }
}

impl From<Option<Hover>> for NavigationOutcome {
    fn from(hover: Option<Hover>) -> Self {
        Self::Hover(hover)
    }
}

impl Drop for FeedbackManager {
    fn drop(&mut self) {
        self.shutdown_all();
    }
}

/// Render a diagnostics block suitable for a tool result. Clean files and absent servers render no
/// block so no spurious diagnostics are emitted.
#[must_use]
pub fn render_feedback_block(shown_path: &str, outcome: &FeedbackOutcome) -> Option<String> {
    match outcome {
        FeedbackOutcome::NoServer => None,
        FeedbackOutcome::Unavailable => Some(format!(
            "LSP unavailable: diagnostics skipped for {shown_path}"
        )),
        FeedbackOutcome::Diagnostics(diagnostics) => {
            if diagnostics.is_empty() {
                return None;
            }
            let errors = diagnostics
                .iter()
                .filter(|diagnostic| diagnostic.severity == "error")
                .count();
            let warnings = diagnostics
                .iter()
                .filter(|diagnostic| diagnostic.severity == "warning")
                .count();
            let mut block = format!(
                "Diagnostics:\nlsp diagnostics for {shown_path}: {errors} error(s), {warnings} warning(s), {} other",
                diagnostics.len().saturating_sub(errors + warnings)
            );
            for diagnostic in diagnostics.iter().take(MAX_RENDERED_DIAGNOSTICS) {
                block.push('\n');
                block.push_str(&render_diagnostic_line(diagnostic));
            }
            if diagnostics.len() > MAX_RENDERED_DIAGNOSTICS {
                block.push_str(&format!(
                    "\n(+{} more diagnostics not shown)",
                    diagnostics.len() - MAX_RENDERED_DIAGNOSTICS
                ));
            }
            Some(block)
        }
    }
}

fn render_diagnostic_line(diagnostic: &FileDiagnostic) -> String {
    let mut message = diagnostic.message.replace('\n', " ");
    if message.chars().count() > MAX_MESSAGE_CHARS {
        message = message.chars().take(MAX_MESSAGE_CHARS).collect();
        message.push('…');
    }
    let code = diagnostic
        .code
        .as_deref()
        .map(|code| format!(" [{code}]"))
        .unwrap_or_default();
    let source = diagnostic
        .source
        .as_deref()
        .map(|source| format!(" ({source})"))
        .unwrap_or_default();
    format!(
        "- {}{code}{source} at line {}, character {}-{}: {message}",
        diagnostic.severity, diagnostic.line, diagnostic.character, diagnostic.end_character
    )
}

fn one_based_to_lsp(line: u32, character: u32) -> Position {
    Position {
        line: line.saturating_sub(1),
        character: character.saturating_sub(1),
    }
}

fn retry_navigation<T, F, P>(
    deadline: Instant,
    mut request: F,
    is_ready: P,
) -> Result<T, crate::LspError>
where
    F: FnMut() -> Result<T, crate::LspError>,
    P: Fn(&T) -> bool,
    T: Default,
{
    loop {
        match request() {
            Ok(result) => {
                if is_ready(&result) {
                    return Ok(result);
                }
                if !sleep_before_navigation_retry(deadline) {
                    return Ok(result);
                }
            }
            Err(error) if is_retryable_navigation_error(&error) => {
                if !sleep_before_navigation_retry(deadline) {
                    // rust-analyzer can keep returning cancellation/readiness errors while a
                    // workspace is still loading. Exhausting the bounded retry budget means this
                    // specific query is not ready yet, not that the server is broken.
                    return Ok(T::default());
                }
            }
            Err(error) => return Err(error),
        }
    }
}

fn sleep_before_navigation_retry(deadline: Instant) -> bool {
    let remaining = deadline
        .checked_duration_since(Instant::now())
        .unwrap_or_default();
    if remaining <= NAVIGATION_RETRY_INTERVAL {
        return false;
    }
    std::thread::sleep(NAVIGATION_RETRY_INTERVAL.min(remaining));
    deadline
        .checked_duration_since(Instant::now())
        .is_some_and(|remaining| remaining > NAVIGATION_RETRY_INTERVAL)
}

fn is_retryable_navigation_error(error: &crate::LspError) -> bool {
    matches!(
        error,
        crate::LspError::ServerError {
            code: LSP_REQUEST_CANCELLED | LSP_CONTENT_MODIFIED | LSP_SERVER_CANCELLED
        }
    )
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum InstallOutcome {
    Available,
    Unavailable,
    NetDisabled,
}

fn ensure_installed(server: &LspServerConfig) -> InstallOutcome {
    if command_exists(server.program()) {
        return InstallOutcome::Available;
    }
    let Some(plan) = server.install() else {
        return InstallOutcome::Unavailable;
    };
    if guard_outbound(&format!("LSP auto-install for {}", server.program())).is_err() {
        return InstallOutcome::NetDisabled;
    }
    let installed = match plan {
        InstallPlan::Npm { package } => Command::new("npm")
            .args(["install", "-g", package.as_str()])
            .status()
            .is_ok_and(|status| status.success()),
        InstallPlan::UvTool { package } => Command::new("uv")
            .args(["tool", "install", package.as_str()])
            .status()
            .is_ok_and(|status| status.success()),
    };
    if installed && command_exists(server.program()) {
        InstallOutcome::Available
    } else {
        InstallOutcome::Unavailable
    }
}

fn command_exists(program: &str) -> bool {
    let program_path = Path::new(program);
    if program_path.components().count() > 1 {
        return program_path.is_file();
    }
    std::env::var_os("PATH").is_some_and(|path| {
        std::env::split_paths(&path).any(|dir| {
            let candidate = dir.join(program);
            candidate.is_file()
        })
    })
}

fn find_running_pid(program: &str) -> Option<u32> {
    let output = Command::new("ps")
        .args(["-eo", "pid=,comm="])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let program_name = Path::new(program)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(program);
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| {
            let mut parts = line.split_whitespace();
            let pid = parts.next()?.parse::<u32>().ok()?;
            let command = parts.next()?;
            (command == program_name).then_some(pid)
        })
        .next()
}

/// Convert an absolute filesystem path to a `file://` URI.
#[must_use]
pub fn path_to_uri(path: &Path) -> String {
    Url::from_file_path(path)
        .map(|url| url.to_string())
        .unwrap_or_else(|()| format!("file://{}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_skips_clean_and_absent_servers() {
        assert_eq!(
            render_feedback_block("a.rs", &FeedbackOutcome::NoServer),
            None
        );
        assert_eq!(
            render_feedback_block("a.rs", &FeedbackOutcome::Diagnostics(Vec::new())),
            None
        );
    }

    #[test]
    fn render_preserves_severity_precise_range_source_and_code() {
        let block = render_feedback_block(
            "bad.py",
            &FeedbackOutcome::Diagnostics(vec![FileDiagnostic {
                severity: "warning",
                line: 1,
                character: 8,
                end_line: 1,
                end_character: 10,
                source: Some("Ruff".to_owned()),
                code: Some("F401".to_owned()),
                message: "`os` imported but unused".to_owned(),
            }]),
        )
        .expect("diagnostics render");
        assert!(block.contains("warning [F401] (Ruff)"), "{block}");
        assert!(block.contains("line 1, character 8-10"), "{block}");
    }

    #[test]
    fn unknown_extension_has_no_server() {
        let mut manager = FeedbackManager::new("/tmp", LspConfigSet::defaults());
        assert_eq!(
            manager.collect(Path::new("/tmp/file.unknownext")),
            FeedbackOutcome::NoServer
        );
    }

    #[test]
    fn retry_navigation_retries_transient_lsp_server_errors() {
        let mut attempts = 0;
        let result = retry_navigation(
            Instant::now() + Duration::from_secs(1),
            || {
                attempts += 1;
                if attempts < 3 {
                    Err(crate::LspError::ServerError { code: -32802 })
                } else {
                    Ok(vec!["ready"])
                }
            },
            |items| !items.is_empty(),
        )
        .expect("transient server cancellations should be retried");

        assert_eq!(result, vec!["ready"]);
        assert_eq!(attempts, 3);
    }

    #[test]
    fn retry_navigation_retries_non_ready_successful_results() {
        let mut attempts = 0;
        let result = retry_navigation(
            Instant::now() + Duration::from_secs(1),
            || {
                attempts += 1;
                if attempts < 3 {
                    Ok(Vec::<&str>::new())
                } else {
                    Ok(vec!["ready"])
                }
            },
            |items| !items.is_empty(),
        )
        .expect("empty but successful navigation results should be retried");

        assert_eq!(result, vec!["ready"]);
        assert_eq!(attempts, 3);
    }

    #[test]
    fn retry_navigation_exhausts_retryable_errors_as_non_fatal_empty_result() {
        let mut attempts = 0;
        let result = retry_navigation(
            Instant::now(),
            || {
                attempts += 1;
                Err(crate::LspError::ServerError {
                    code: LSP_CONTENT_MODIFIED,
                })
            },
            |items: &Vec<&str>| !items.is_empty(),
        )
        .expect("retryable readiness errors should not become fatal navigation errors");

        assert!(result.is_empty());
        assert_eq!(attempts, 1);
    }

    #[test]
    fn retryable_navigation_errors_are_lsp_cancellation_codes() {
        for code in [
            LSP_REQUEST_CANCELLED,
            LSP_CONTENT_MODIFIED,
            LSP_SERVER_CANCELLED,
        ] {
            assert!(is_retryable_navigation_error(
                &crate::LspError::ServerError { code }
            ));
        }
        assert!(!is_retryable_navigation_error(
            &crate::LspError::ServerError { code: -32601 }
        ));
        assert!(!is_retryable_navigation_error(&crate::LspError::Closed));
    }

    #[test]
    fn retry_navigation_does_not_retry_fatal_lsp_errors() {
        let mut attempts = 0;
        let result: Result<Vec<&str>, crate::LspError> = retry_navigation(
            Instant::now() + Duration::from_secs(1),
            || {
                attempts += 1;
                Err(crate::LspError::ServerError { code: -32601 })
            },
            |items: &Vec<&str>| !items.is_empty(),
        );

        assert_eq!(result, Err(crate::LspError::ServerError { code: -32601 }));
        assert_eq!(attempts, 1);
    }
}
