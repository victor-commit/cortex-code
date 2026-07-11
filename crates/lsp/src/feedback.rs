//! Post-mutation diagnostics feedback built on real language servers.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::OpenOptions;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use nix::fcntl::{Flock, FlockArg};
use redaction::guard_outbound;
use url::Url;

use crate::client::{LspClient, LspProcessIdentity};
use crate::config::{InstallPlan, LspConfigSet, LspServerConfig};
use crate::diagnostics::{DiagnosticSeverity, Position};
use crate::navigation::{
    CallHierarchyItem, DocumentSymbol, Hover, IncomingCall, Location, OutgoingCall,
    SymbolInformation,
};
use crate::protocol::InitializeParams;

/// Total wall-clock budget for one post-mutation LSP diagnostic collection.
pub const FEEDBACK_BUDGET: Duration = Duration::from_millis(4500);
/// Maximum diagnostics rendered into a tool result.
pub const MAX_RENDERED_DIAGNOSTICS: usize = 20;
/// Maximum characters from one diagnostic message.
pub const MAX_MESSAGE_CHARS: usize = 500;
const NAVIGATION_RETRY_INTERVAL: Duration = Duration::from_millis(150);
const PULL_DIAGNOSTICS_WAIT_BEFORE_FALLBACK: Duration = Duration::from_millis(1200);
const LSP_CRASH_STATE_DIR: &str = "lsp/crash-state";
const LSP_RUNTIME_STATE_DIR: &str = "lsp/runtime-state";
static RUNTIME_STATE_UPDATE_MUTEX: OnceLock<Mutex<()>> = OnceLock::new();
// LSP cancellation/readiness errors that rust-analyzer may return while a just-opened workspace is
// still loading or while it invalidates an in-flight query after didOpen/didChange. Treat these as
// retryable for navigation only, without weakening fatal transport/process handling.
const LSP_REQUEST_CANCELLED: i64 = -32800;
const LSP_CONTENT_MODIFIED: i64 = -32801;
const LSP_SERVER_CANCELLED: i64 = -32802;
const LSP_METHOD_NOT_FOUND: i64 = -32601;

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

/// One workspace diagnostic entry with a stable file path.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkspaceFileDiagnostics {
    /// Stable file path for this diagnostic batch.
    pub path: String,
    /// Diagnostics for the file.
    pub diagnostics: Vec<FileDiagnostic>,
}

/// Result of one workspace-level pull diagnostics request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WorkspaceDiagnosticsOutcome {
    /// No server applies, binary is missing, install was disabled, or startup failed.
    NoServer,
    /// A configured server exists but does not support workspace pull diagnostics.
    Unsupported,
    /// A previously running server died or stopped responding.
    Unavailable,
    /// Diagnostics currently known across files in the workspace.
    Diagnostics(Vec<WorkspaceFileDiagnostics>),
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
    /// Go to implementation from a symbol use/definition site.
    Implementation {
        file_path: PathBuf,
        line: u32,
        character: u32,
    },
    /// Prepare call hierarchy items for a symbol.
    PrepareCallHierarchy {
        file_path: PathBuf,
        line: u32,
        character: u32,
    },
    /// Resolve incoming call hierarchy edges for a symbol.
    IncomingCalls {
        file_path: PathBuf,
        line: u32,
        character: u32,
    },
    /// Resolve outgoing call hierarchy edges for a symbol.
    OutgoingCalls {
        file_path: PathBuf,
        line: u32,
        character: u32,
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
    /// Prepared call hierarchy items.
    CallHierarchyItems(Vec<CallHierarchyItem>),
    /// Incoming call hierarchy edges.
    IncomingCalls(Vec<IncomingCall>),
    /// Outgoing call hierarchy edges.
    OutgoingCalls(Vec<OutgoingCall>),
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
    /// Optional auto-install guidance when the server binary is missing.
    pub install_hint: Option<String>,
}

/// Coarse state for one managed server.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ServerRunState {
    /// Explicitly disabled in config.
    Disabled,
    /// Missing locally but auto-installable when network policy allows.
    Installing,
    /// Running with OS PID.
    Running(u32),
    /// Installed and executable, but not currently running.
    Available,
    /// Missing and not installable, blocked by no-net, or startup unavailable.
    Unavailable,
    /// Was running and then stopped or crashed.
    Crashed,
}

impl ServerRunState {
    /// Stable lowercase label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Disabled => "disabled",
            Self::Installing => "installing",
            Self::Running(_) => "running",
            Self::Available => "available",
            Self::Unavailable => "unavailable",
            Self::Crashed => "crashed",
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

struct DurableCrashState {
    path: Option<PathBuf>,
    crashed_languages: BTreeSet<String>,
}

impl DurableCrashState {
    fn load(root: &Path) -> Self {
        let path = crash_state_path(root);
        let crashed_languages = path
            .as_ref()
            .and_then(|path| std::fs::read(path).ok())
            .and_then(|bytes| serde_json::from_slice::<Vec<String>>(&bytes).ok())
            .map(|entries| entries.into_iter().collect())
            .unwrap_or_default();
        Self {
            path,
            crashed_languages,
        }
    }

    fn is_crashed(&self, language_id: &str) -> bool {
        self.crashed_languages.contains(language_id)
    }

    fn mark_crashed(&mut self, language_id: &str) {
        if self.crashed_languages.insert(language_id.to_owned()) {
            self.persist();
        }
    }

    fn clear_crashed(&mut self, language_id: &str) {
        if self.crashed_languages.remove(language_id) {
            self.persist();
        }
    }

    fn persist(&self) {
        let Some(path) = self.path.as_ref() else {
            return;
        };
        if let Some(parent) = path.parent() {
            if std::fs::create_dir_all(parent).is_err() {
                return;
            }
        }
        let mut values = self
            .crashed_languages
            .iter()
            .cloned()
            .collect::<Vec<String>>();
        values.sort();
        let Ok(bytes) = serde_json::to_vec_pretty(&values) else {
            return;
        };
        let _ = std::fs::write(path, bytes);
    }
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
struct ManagedServer {
    pid: u32,
    /// `None` is a legacy PID-only record and must fail closed: it cannot establish ownership
    /// after a process exits or the operating system reuses its PID.
    #[serde(default)]
    start_time: Option<u64>,
}

impl ManagedServer {
    fn is_live(&self) -> bool {
        self.start_time.is_some_and(|start_time| {
            LspProcessIdentity::from_parts(self.pid, start_time).is_live()
        })
    }
}

/// Per-workspace ownership records let a status-only Cortex process identify a server another
/// still-running Cortex process launched. This intentionally avoids guessing from an unrelated
/// process with the same executable name.
struct RuntimeServerState {
    path: Option<PathBuf>,
    servers: BTreeMap<String, ManagedServer>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RegisteredServer {
    None,
    Running(u32),
    Stale,
}

impl RuntimeServerState {
    fn load(root: &Path) -> Self {
        let path = runtime_state_path(root);
        let servers = path
            .as_ref()
            .map_or_else(BTreeMap::new, |path| read_runtime_servers(path));
        Self { path, servers }
    }

    fn status_for(&mut self, language_id: &str) -> RegisteredServer {
        self.reload();
        let Some(server) = self.servers.get(language_id).cloned() else {
            return RegisteredServer::None;
        };
        if server.is_live() {
            return RegisteredServer::Running(server.pid);
        }
        if self.remove_if_matches(language_id, &server) {
            return RegisteredServer::Stale;
        }
        self.servers
            .get(language_id)
            .filter(|replacement| replacement.is_live())
            .map_or(RegisteredServer::Stale, |replacement| {
                RegisteredServer::Running(replacement.pid)
            })
    }

    fn register(&mut self, language_id: &str, identity: Option<LspProcessIdentity>) {
        let Some(identity) = identity else {
            return;
        };
        let server = ManagedServer {
            pid: identity.pid(),
            start_time: Some(identity.start_time()),
        };
        self.update(|servers| {
            let changed = servers.get(language_id) != Some(&server);
            if changed {
                servers.insert(language_id.to_owned(), server);
            }
            ((), changed)
        });
    }

    fn unregister(&mut self, language_id: &str, identity: Option<LspProcessIdentity>) {
        let Some(identity) = identity else {
            return;
        };
        let expected = ManagedServer {
            pid: identity.pid(),
            start_time: Some(identity.start_time()),
        };
        let _ = self.remove_if_matches(language_id, &expected);
    }

    fn reload(&mut self) {
        self.update(|_| ((), false));
    }

    fn remove_if_matches(&mut self, language_id: &str, expected: &ManagedServer) -> bool {
        self.update(|servers| {
            let matches = servers.get(language_id) == Some(expected);
            if matches {
                servers.remove(language_id);
            }
            (matches, matches)
        })
    }

    fn update<T>(
        &mut self,
        operation: impl FnOnce(&mut BTreeMap<String, ManagedServer>) -> (T, bool),
    ) -> T {
        let Some(path) = self.path.clone() else {
            let (result, _) = operation(&mut self.servers);
            return result;
        };
        let Some(parent) = path.parent() else {
            let (result, _) = operation(&mut self.servers);
            return result;
        };
        if std::fs::create_dir_all(parent).is_err() {
            let (result, _) = operation(&mut self.servers);
            return result;
        }

        let guard = RUNTIME_STATE_UPDATE_MUTEX
            .get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let lock_path = path.with_extension("lock");
        let Ok(lock_file) = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(lock_path)
        else {
            drop(guard);
            let (result, _) = operation(&mut self.servers);
            return result;
        };
        let Ok(_lock_file) = Flock::lock(lock_file, FlockArg::LockExclusive) else {
            drop(guard);
            let (result, _) = operation(&mut self.servers);
            return result;
        };

        let mut servers = read_runtime_servers(&path);
        let (result, changed) = operation(&mut servers);
        if changed {
            persist_runtime_servers(&path, &servers);
        }
        self.servers = servers;
        drop(guard);
        result
    }
}

fn read_runtime_servers(path: &Path) -> BTreeMap<String, ManagedServer> {
    std::fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

fn persist_runtime_servers(path: &Path, servers: &BTreeMap<String, ManagedServer>) {
    let Some(parent) = path.parent() else {
        return;
    };
    if std::fs::create_dir_all(parent).is_err() {
        return;
    }
    let Ok(bytes) = serde_json::to_vec(servers) else {
        return;
    };
    let temporary = path.with_extension("tmp");
    if std::fs::write(&temporary, bytes).is_ok() {
        let _ = std::fs::rename(temporary, path);
    }
}

/// Lazy per-language server pool plus bounded diagnostics collection.
pub struct FeedbackManager {
    root: PathBuf,
    config: LspConfigSet,
    servers: BTreeMap<String, ServerSlot>,
    crash_state: DurableCrashState,
    runtime_state: RuntimeServerState,
    budget: Duration,
}

impl FeedbackManager {
    /// Create a manager rooted at `root`.
    #[must_use]
    pub fn new(root: impl Into<PathBuf>, config: LspConfigSet) -> Self {
        let root = root.into();
        Self {
            crash_state: DurableCrashState::load(&root),
            runtime_state: RuntimeServerState::load(&root),
            root,
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
        self.collect_until(path, Instant::now() + self.budget)
    }

    fn collect_until(&mut self, path: &Path, deadline: Instant) -> FeedbackOutcome {
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
        let wait_for_push = if client.supports_document_diagnostics_pull() {
            remaining.min(PULL_DIAGNOSTICS_WAIT_BEFORE_FALLBACK)
        } else {
            remaining
        };
        let fresh = match client.poll_fresh_diagnostics(
            &uri,
            document_version,
            baseline_generation,
            wait_for_push,
        ) {
            Ok(fresh) => fresh,
            Err(_) => {
                self.mark_broken(&language_id);
                return FeedbackOutcome::Unavailable;
            }
        };
        let should_pull = client.supports_document_diagnostics_pull()
            && fresh
                .as_ref()
                .is_none_or(|diagnostics| diagnostics.is_empty());
        let pulled = if should_pull {
            match client.document_diagnostics_until(&uri, deadline) {
                Ok(diagnostics) => Some(diagnostics),
                Err(crate::LspError::ServerError {
                    code: LSP_METHOD_NOT_FOUND,
                }) => None,
                Err(_) => {
                    self.mark_broken(&language_id);
                    return FeedbackOutcome::Unavailable;
                }
            }
        } else {
            None
        };
        let diagnostics = if let Some(diagnostics) = fresh.as_ref().filter(|d| !d.is_empty()) {
            diagnostics.clone()
        } else if let Some(pulled) = pulled {
            pulled
        } else if let Some(fresh) = fresh {
            fresh
        } else {
            client.diagnostics().for_file(&uri).to_vec()
        }
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

    /// Workspace-level pull diagnostics for files handled by the matching language server.
    pub fn collect_workspace(&mut self, path: &Path) -> WorkspaceDiagnosticsOutcome {
        let deadline = Instant::now() + self.budget;
        let absolute = if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.root.join(path)
        };
        let Some(server) = self.config.resolve_for_path(&absolute).cloned() else {
            return WorkspaceDiagnosticsOutcome::NoServer;
        };
        let language_id = server.language_id().to_owned();

        match self.servers.get(&language_id) {
            Some(ServerSlot::Missing | ServerSlot::Failed) => {
                return WorkspaceDiagnosticsOutcome::NoServer;
            }
            Some(ServerSlot::Broken) => return WorkspaceDiagnosticsOutcome::Unavailable,
            Some(ServerSlot::Running { .. }) | None => {}
        }
        if !self.servers.contains_key(&language_id) && !self.start_server(&server, deadline) {
            return match self.servers.get(&language_id) {
                Some(ServerSlot::Missing) => WorkspaceDiagnosticsOutcome::NoServer,
                Some(ServerSlot::Failed | ServerSlot::Broken) => {
                    WorkspaceDiagnosticsOutcome::Unavailable
                }
                Some(ServerSlot::Running { .. }) | None => WorkspaceDiagnosticsOutcome::NoServer,
            };
        }

        if self.ensure_document(path, deadline).is_err() {
            return match self.servers.get(&language_id) {
                Some(ServerSlot::Broken | ServerSlot::Failed) => {
                    WorkspaceDiagnosticsOutcome::Unavailable
                }
                _ => WorkspaceDiagnosticsOutcome::NoServer,
            };
        }

        let supports_workspace_pull = matches!(
            self.servers.get(&language_id),
            Some(ServerSlot::Running { client, .. }) if client.supports_workspace_diagnostics_pull()
        );
        if !supports_workspace_pull {
            return self.collect_workspace_via_document_pull(&language_id, &absolute, deadline);
        }

        let Some(ServerSlot::Running { client, .. }) = self.servers.get_mut(&language_id) else {
            return WorkspaceDiagnosticsOutcome::Unavailable;
        };
        let entries = match client.workspace_diagnostics_until(deadline) {
            Ok(entries) => entries,
            Err(crate::LspError::ServerError {
                code: LSP_METHOD_NOT_FOUND,
            }) => return WorkspaceDiagnosticsOutcome::Unsupported,
            Err(_) => {
                self.mark_broken(&language_id);
                return WorkspaceDiagnosticsOutcome::Unavailable;
            }
        };

        let mut diagnostics = entries
            .into_iter()
            .filter_map(|entry| {
                if entry.diagnostics.is_empty() {
                    return None;
                }
                Some(WorkspaceFileDiagnostics {
                    path: uri_to_stable_path(&entry.uri),
                    diagnostics: entry
                        .diagnostics
                        .into_iter()
                        .map(|diagnostic| FileDiagnostic {
                            severity: diagnostic
                                .severity
                                .map_or("note", DiagnosticSeverity::label),
                            line: diagnostic.range.start.line.saturating_add(1),
                            character: diagnostic.range.start.character.saturating_add(1),
                            end_line: diagnostic.range.end.line.saturating_add(1),
                            end_character: diagnostic.range.end.character.saturating_add(1),
                            source: diagnostic.source,
                            code: diagnostic.code,
                            message: diagnostic.message,
                        })
                        .collect(),
                })
            })
            .collect::<Vec<_>>();
        diagnostics.sort_by(|left, right| left.path.cmp(&right.path));
        WorkspaceDiagnosticsOutcome::Diagnostics(diagnostics)
    }

    fn collect_workspace_via_document_pull(
        &mut self,
        language_id: &str,
        requested_path: &Path,
        deadline: Instant,
    ) -> WorkspaceDiagnosticsOutcome {
        let mut files = self.workspace_language_files(language_id);
        if requested_path.is_file()
            && self
                .config
                .resolve_for_path(requested_path)
                .is_some_and(|server| server.language_id() == language_id)
            && !files.iter().any(|candidate| candidate == requested_path)
        {
            files.push(requested_path.to_path_buf());
        }
        files.sort();
        files.dedup();

        let mut workspace = Vec::new();
        for file in files {
            if deadline
                .checked_duration_since(Instant::now())
                .is_none_or(|remaining| remaining.is_zero())
            {
                return WorkspaceDiagnosticsOutcome::Unavailable;
            }
            match self.collect_until(&file, deadline) {
                FeedbackOutcome::Diagnostics(diagnostics) => {
                    if diagnostics.is_empty() {
                        continue;
                    }
                    workspace.push(WorkspaceFileDiagnostics {
                        path: file.to_string_lossy().into_owned(),
                        diagnostics,
                    });
                }
                FeedbackOutcome::NoServer => {}
                FeedbackOutcome::Unavailable => return WorkspaceDiagnosticsOutcome::Unavailable,
            }
        }
        workspace.sort_by(|left, right| left.path.cmp(&right.path));
        WorkspaceDiagnosticsOutcome::Diagnostics(workspace)
    }

    fn workspace_language_files(&self, language_id: &str) -> Vec<PathBuf> {
        let mut files = Vec::new();
        self.collect_workspace_language_files(&self.root, language_id, &mut files);
        files
    }

    fn collect_workspace_language_files(
        &self,
        root: &Path,
        language_id: &str,
        files: &mut Vec<PathBuf>,
    ) {
        let Ok(entries) = std::fs::read_dir(root) else {
            return;
        };
        for entry in entries.flatten() {
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            let path = entry.path();
            if file_type.is_symlink() {
                continue;
            }
            if file_type.is_dir() {
                if should_skip_workspace_dir(&path) {
                    continue;
                }
                self.collect_workspace_language_files(&path, language_id, files);
                continue;
            }
            if !file_type.is_file() {
                continue;
            }
            if self
                .config
                .resolve_for_path(&path)
                .is_some_and(|server| server.language_id() == language_id)
            {
                files.push(path);
            }
        }
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
            NavigationRequest::Implementation {
                file_path,
                line,
                character,
            } => self.with_document(&file_path, deadline, |client, uri| {
                let position = one_based_to_lsp(line, character);
                retry_navigation(
                    deadline,
                    || client.goto_implementation(uri, position),
                    |locations| !locations.is_empty(),
                )
                .map(NavigationOutcome::Locations)
            }),
            NavigationRequest::PrepareCallHierarchy {
                file_path,
                line,
                character,
            } => self.with_document(&file_path, deadline, |client, uri| {
                let position = one_based_to_lsp(line, character);
                retry_navigation(
                    deadline,
                    || client.prepare_call_hierarchy(uri, position),
                    |items| !items.is_empty(),
                )
                .map(NavigationOutcome::CallHierarchyItems)
            }),
            NavigationRequest::IncomingCalls {
                file_path,
                line,
                character,
            } => self.with_document(&file_path, deadline, |client, uri| {
                let position = one_based_to_lsp(line, character);
                retry_navigation(
                    deadline,
                    || client.incoming_calls(uri, position),
                    |calls| !calls.is_empty(),
                )
                .map(NavigationOutcome::IncomingCalls)
            }),
            NavigationRequest::OutgoingCalls {
                file_path,
                line,
                character,
            } => self.with_document(&file_path, deadline, |client, uri| {
                let position = one_based_to_lsp(line, character);
                retry_navigation(
                    deadline,
                    || client.outgoing_calls(uri, position),
                    |calls| !calls.is_empty(),
                )
                .map(NavigationOutcome::OutgoingCalls)
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
        let no_net = redaction::network_disabled();
        let mut rows = Vec::new();
        for server in self.config.servers() {
            let language_id = server.language_id().to_owned();
            let install_hint = status_install_hint(server, no_net);
            let mut mark_crashed = false;
            let mut clear_crashed = false;
            let state = match self.servers.get_mut(server.language_id()) {
                _ if !server.is_enabled() => ServerRunState::Disabled,
                None => match self.runtime_state.status_for(server.language_id()) {
                    RegisteredServer::Running(pid) => {
                        clear_crashed = true;
                        ServerRunState::Running(pid)
                    }
                    RegisteredServer::Stale => {
                        mark_crashed = true;
                        ServerRunState::Crashed
                    }
                    RegisteredServer::None => match install_readiness(server, no_net) {
                        InstallReadiness::Available(_) => {
                            if self.crash_state.is_crashed(server.language_id()) {
                                ServerRunState::Crashed
                            } else {
                                ServerRunState::Available
                            }
                        }
                        InstallReadiness::Installable => ServerRunState::Installing,
                        InstallReadiness::NoPlan
                        | InstallReadiness::NetDisabled
                        | InstallReadiness::MissingPrerequisite { .. } => {
                            ServerRunState::Unavailable
                        }
                    },
                },
                Some(ServerSlot::Running { client, .. }) => {
                    if client.is_running() {
                        self.runtime_state
                            .register(server.language_id(), client.process_identity());
                        clear_crashed = true;
                        ServerRunState::Running(client.child_id())
                    } else {
                        self.runtime_state
                            .unregister(server.language_id(), client.process_identity());
                        mark_crashed = true;
                        ServerRunState::Crashed
                    }
                }
                Some(ServerSlot::Missing) => match install_readiness(server, no_net) {
                    InstallReadiness::Installable => ServerRunState::Installing,
                    InstallReadiness::Available(_) => {
                        if self.crash_state.is_crashed(server.language_id()) {
                            ServerRunState::Crashed
                        } else {
                            ServerRunState::Available
                        }
                    }
                    InstallReadiness::NoPlan
                    | InstallReadiness::NetDisabled
                    | InstallReadiness::MissingPrerequisite { .. } => ServerRunState::Unavailable,
                },
                Some(ServerSlot::Failed | ServerSlot::Broken) => {
                    mark_crashed = true;
                    ServerRunState::Crashed
                }
            };
            if mark_crashed {
                self.crash_state.mark_crashed(&language_id);
            }
            if clear_crashed {
                self.crash_state.clear_crashed(&language_id);
            };
            rows.push(ServerStatus {
                language_id,
                program: server.program().to_owned(),
                state,
                install_hint,
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
                let identity = client.process_identity();
                let _ = client.shutdown();
                let _ = client.exit();
                self.runtime_state.unregister(&language_id, identity);
                self.crash_state.clear_crashed(&language_id);
            }
        }
    }

    fn start_server(&mut self, server: &LspServerConfig, deadline: Instant) -> bool {
        match ensure_installed(server, deadline) {
            InstallOutcome::Available => {}
            InstallOutcome::Unavailable
            | InstallOutcome::NetDisabled
            | InstallOutcome::MissingPrerequisite => {
                self.servers
                    .insert(server.language_id().to_owned(), ServerSlot::Missing);
                return false;
            }
        }
        let Some(program_path) = resolve_server_program_until(server, deadline) else {
            self.servers
                .insert(server.language_id().to_owned(), ServerSlot::Missing);
            return false;
        };

        let remaining = deadline
            .checked_duration_since(Instant::now())
            .unwrap_or_default();
        if remaining.is_zero() {
            self.servers
                .insert(server.language_id().to_owned(), ServerSlot::Failed);
            self.crash_state.mark_crashed(server.language_id());
            return false;
        }
        let command = server
            .to_command_with_program(program_path.to_string_lossy().into_owned())
            .with_request_timeout(remaining);
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
                self.crash_state.clear_crashed(server.language_id());
                self.runtime_state
                    .register(server.language_id(), client.process_identity());
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
                self.crash_state.mark_crashed(server.language_id());
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
        if let Some(ServerSlot::Running { mut client, .. }) = self.servers.remove(language_id) {
            let identity = client.process_identity();
            client.force_reap();
            self.runtime_state.unregister(language_id, identity);
        }
        self.servers
            .insert(language_id.to_owned(), ServerSlot::Broken);
        self.crash_state.mark_crashed(language_id);
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
    let mut final_quick_retry_used = false;
    loop {
        match request() {
            Ok(result) => {
                if is_ready(&result) {
                    return Ok(result);
                }
                if !sleep_before_navigation_retry(deadline, &mut final_quick_retry_used) {
                    return Ok(result);
                }
            }
            Err(error) if is_retryable_navigation_error(&error) => {
                if !sleep_before_navigation_retry(deadline, &mut final_quick_retry_used) {
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

fn sleep_before_navigation_retry(deadline: Instant, final_quick_retry_used: &mut bool) -> bool {
    let remaining = deadline
        .checked_duration_since(Instant::now())
        .unwrap_or_default();
    if remaining.is_zero() {
        return false;
    }
    if remaining <= NAVIGATION_RETRY_INTERVAL {
        if *final_quick_retry_used {
            return false;
        }
        // When the remaining budget is under the normal retry interval, allow one final quick
        // retry instead of stopping early.
        *final_quick_retry_used = true;
        return true;
    }
    std::thread::sleep(NAVIGATION_RETRY_INTERVAL);
    deadline
        .checked_duration_since(Instant::now())
        .is_some_and(|remaining| !remaining.is_zero())
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
    MissingPrerequisite,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum InstallReadiness {
    Available(PathBuf),
    Installable,
    NoPlan,
    NetDisabled,
    MissingPrerequisite {
        installer: &'static str,
        command: String,
    },
}

fn ensure_installed(server: &LspServerConfig, deadline: Instant) -> InstallOutcome {
    match install_readiness_until(server, redaction::network_disabled(), deadline) {
        InstallReadiness::Available(_) => return InstallOutcome::Available,
        InstallReadiness::NoPlan => return InstallOutcome::Unavailable,
        InstallReadiness::NetDisabled => return InstallOutcome::NetDisabled,
        InstallReadiness::MissingPrerequisite { .. } => {
            return InstallOutcome::MissingPrerequisite;
        }
        InstallReadiness::Installable => {}
    };
    let Some(plan) = server.install() else {
        return InstallOutcome::Unavailable;
    };
    if guard_outbound(&format!("LSP auto-install for {}", server.program())).is_err() {
        return InstallOutcome::NetDisabled;
    }
    eprintln!(
        "lsp auto-install: server={} source={} target={} version=latest",
        server.program(),
        plan.source(),
        plan.target()
    );
    let installed = match plan {
        InstallPlan::Npm { package } => run_installer_until(
            Command::new("npm").args(["install", "-g", package.as_str()]),
            deadline,
        ),
        InstallPlan::UvTool { package } => run_installer_until(
            Command::new("uv").args(["tool", "install", package.as_str()]),
            deadline,
        ),
        InstallPlan::Cargo { package } => run_installer_until(
            Command::new("cargo").args(["install", "--locked", package.as_str()]),
            deadline,
        ),
        InstallPlan::Go { module } => run_installer_until(
            Command::new("go").args(["install", &format!("{module}@latest")]),
            deadline,
        ),
        InstallPlan::DotnetTool { package } => {
            run_installer_until(
                Command::new("dotnet").args(["tool", "install", "--global", package.as_str()]),
                deadline,
            ) || run_installer_until(
                Command::new("dotnet").args(["tool", "update", "--global", package.as_str()]),
                deadline,
            )
        }
        InstallPlan::Gem { package } => run_installer_until(
            Command::new("gem").args(["install", package.as_str()]),
            deadline,
        ),
    };
    if installed
        && matches!(
            install_readiness_until(server, redaction::network_disabled(), deadline),
            InstallReadiness::Available(_)
        )
    {
        InstallOutcome::Available
    } else {
        InstallOutcome::Unavailable
    }
}

fn run_installer_until(command: &mut Command, deadline: Instant) -> bool {
    if deadline
        .checked_duration_since(Instant::now())
        .is_none_or(|remaining| remaining.is_zero())
    {
        return false;
    }
    let Ok(mut child) = command.spawn() else {
        return false;
    };
    wait_for_child_until(&mut child, deadline).is_some_and(|status| status.success())
}

fn wait_for_child_until(child: &mut Child, deadline: Instant) -> Option<std::process::ExitStatus> {
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Some(status),
            Ok(None) => {
                let remaining = deadline
                    .checked_duration_since(Instant::now())
                    .unwrap_or_default();
                if remaining.is_zero() {
                    reap_exact_child(child);
                    return None;
                }
                std::thread::sleep(remaining.min(Duration::from_millis(10)));
            }
            Err(_) => {
                reap_exact_child(child);
                return None;
            }
        }
    }
}

fn reap_exact_child(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

fn install_readiness(server: &LspServerConfig, no_net: bool) -> InstallReadiness {
    if let Some(program) = resolve_server_program(server) {
        return InstallReadiness::Available(program);
    }
    let Some(plan) = server.install() else {
        return InstallReadiness::NoPlan;
    };
    if no_net {
        return InstallReadiness::NetDisabled;
    }
    if let Some(installer) = install_prerequisite(plan).filter(|program| !command_exists(program)) {
        return InstallReadiness::MissingPrerequisite {
            installer,
            command: plan.description(),
        };
    }
    InstallReadiness::Installable
}

fn install_readiness_until(
    server: &LspServerConfig,
    no_net: bool,
    deadline: Instant,
) -> InstallReadiness {
    if let Some(program) = resolve_server_program_until(server, deadline) {
        return InstallReadiness::Available(program);
    }
    let Some(plan) = server.install() else {
        return InstallReadiness::NoPlan;
    };
    if no_net {
        return InstallReadiness::NetDisabled;
    }
    if let Some(installer) = install_prerequisite(plan).filter(|program| !command_exists(program)) {
        return InstallReadiness::MissingPrerequisite {
            installer,
            command: plan.description(),
        };
    }
    InstallReadiness::Installable
}

fn status_install_hint(server: &LspServerConfig, no_net: bool) -> Option<String> {
    let plan = server.install()?;
    let base = plan.description();
    match install_readiness(server, no_net) {
        InstallReadiness::NetDisabled => Some(format!("{base} (blocked: CORTEX_NO_NET=1)")),
        InstallReadiness::MissingPrerequisite { installer, .. } => {
            Some(format!("{base} (blocked: requires `{installer}` on PATH)"))
        }
        InstallReadiness::Available(_)
        | InstallReadiness::Installable
        | InstallReadiness::NoPlan => Some(base),
    }
}

pub fn no_server_guidance(server: &LspServerConfig, no_net: bool) -> Option<String> {
    if no_net && server.install().is_some() {
        return Some(format!(
            "CORTEX_NO_NET=1 prevents auto-installing a missing language server ({})",
            server.program()
        ));
    }
    match install_readiness(server, false) {
        InstallReadiness::MissingPrerequisite { installer, command } => Some(format!(
            "auto-install for {} requires `{installer}` on PATH ({command})",
            server.program()
        )),
        InstallReadiness::Available(_)
        | InstallReadiness::Installable
        | InstallReadiness::NoPlan
        | InstallReadiness::NetDisabled => None,
    }
}

fn crash_state_path(root: &Path) -> Option<PathBuf> {
    let workspace_key = checkpoints::workspace_key(root);
    crash_state_base_dir().map(|base| base.join(format!("{workspace_key}.json")))
}

fn runtime_state_path(root: &Path) -> Option<PathBuf> {
    let workspace_key = checkpoints::workspace_key(root);
    runtime_state_base_dir().map(|base| base.join(format!("{workspace_key}.json")))
}

fn crash_state_base_dir() -> Option<PathBuf> {
    lsp_state_base_dir(LSP_CRASH_STATE_DIR)
}

fn runtime_state_base_dir() -> Option<PathBuf> {
    lsp_state_base_dir(LSP_RUNTIME_STATE_DIR)
}

fn lsp_state_base_dir(kind: &str) -> Option<PathBuf> {
    if let Some(data_dir) = non_empty_var("CORTEX_DATA_DIR") {
        return Some(data_dir.join(kind));
    }
    if let Some(db_path) = non_empty_var("CORTEX_DB_PATH") {
        if let Some(parent) = db_path.parent() {
            return Some(parent.join(kind));
        }
    }
    if let Some(xdg_data_home) = non_empty_var("XDG_DATA_HOME") {
        return Some(xdg_data_home.join("cortex").join(kind));
    }
    non_empty_var("HOME").map(|home| home.join(".local").join("share").join("cortex").join(kind))
}

fn non_empty_var(name: &str) -> Option<PathBuf> {
    std::env::var_os(name)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

fn resolve_server_program(server: &LspServerConfig) -> Option<PathBuf> {
    find_program(server.program()).or_else(|| match server.install() {
        Some(InstallPlan::Go { .. }) => resolve_go_program(server.program()),
        _ => None,
    })
}

fn resolve_server_program_until(server: &LspServerConfig, deadline: Instant) -> Option<PathBuf> {
    find_program(server.program()).or_else(|| match server.install() {
        Some(InstallPlan::Go { .. }) => resolve_go_program_until(server.program(), deadline),
        _ => None,
    })
}

fn command_exists(program: &str) -> bool {
    find_program(program).is_some()
}

fn find_program(program: &str) -> Option<PathBuf> {
    let program_path = Path::new(program);
    if program_path.components().count() > 1 {
        return program_path.is_file().then(|| program_path.to_path_buf());
    }
    std::env::var_os("PATH").and_then(|path| {
        std::env::split_paths(&path).find_map(|dir| {
            executable_names(program)
                .into_iter()
                .map(|name| dir.join(name))
                .find(|candidate| candidate.is_file())
        })
    })
}

fn install_prerequisite(plan: &InstallPlan) -> Option<&'static str> {
    match plan {
        InstallPlan::Npm { .. } => Some("npm"),
        InstallPlan::UvTool { .. } => Some("uv"),
        InstallPlan::Cargo { .. } => Some("cargo"),
        InstallPlan::Go { .. } => Some("go"),
        InstallPlan::DotnetTool { .. } => Some("dotnet"),
        InstallPlan::Gem { .. } => Some("gem"),
    }
}

fn resolve_go_program(program: &str) -> Option<PathBuf> {
    go_install_bin_dirs().into_iter().find_map(|dir| {
        executable_names(program)
            .into_iter()
            .map(|name| dir.join(name))
            .find(|candidate| candidate.is_file())
    })
}

fn resolve_go_program_until(program: &str, deadline: Instant) -> Option<PathBuf> {
    go_install_bin_dirs_until(deadline)
        .into_iter()
        .find_map(|dir| {
            executable_names(program)
                .into_iter()
                .map(|name| dir.join(name))
                .find(|candidate| candidate.is_file())
        })
}

fn go_install_bin_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    push_unique_dir(&mut dirs, std::env::var_os("GOBIN").map(PathBuf::from));
    push_unique_dir(
        &mut dirs,
        go_env_value("GOBIN").map(|value| PathBuf::from(value.trim())),
    );
    if let Some(gopath) = std::env::var_os("GOPATH") {
        for entry in std::env::split_paths(&gopath) {
            push_unique_dir(&mut dirs, Some(entry.join("bin")));
        }
    }
    if let Some(gopath) = go_env_value("GOPATH") {
        let gopath = std::ffi::OsString::from(gopath.trim());
        for entry in std::env::split_paths(&gopath) {
            push_unique_dir(&mut dirs, Some(entry.join("bin")));
        }
    }
    push_unique_dir(
        &mut dirs,
        std::env::var_os("HOME").map(|home| PathBuf::from(home).join("go").join("bin")),
    );
    dirs
}

fn go_install_bin_dirs_until(deadline: Instant) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    push_unique_dir(&mut dirs, std::env::var_os("GOBIN").map(PathBuf::from));
    push_unique_dir(
        &mut dirs,
        go_env_value_until("GOBIN", deadline).map(|value| PathBuf::from(value.trim())),
    );
    if let Some(gopath) = std::env::var_os("GOPATH") {
        for entry in std::env::split_paths(&gopath) {
            push_unique_dir(&mut dirs, Some(entry.join("bin")));
        }
    }
    if let Some(gopath) = go_env_value_until("GOPATH", deadline) {
        let gopath = std::ffi::OsString::from(gopath.trim());
        for entry in std::env::split_paths(&gopath) {
            push_unique_dir(&mut dirs, Some(entry.join("bin")));
        }
    }
    push_unique_dir(
        &mut dirs,
        std::env::var_os("HOME").map(|home| PathBuf::from(home).join("go").join("bin")),
    );
    dirs
}

fn go_env_value(variable: &str) -> Option<String> {
    if !command_exists("go") {
        return None;
    }
    let output = Command::new("go").args(["env", variable]).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let value = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    (!value.is_empty()).then_some(value)
}

fn go_env_value_until(variable: &str, deadline: Instant) -> Option<String> {
    if !command_exists("go") {
        return None;
    }
    if deadline
        .checked_duration_since(Instant::now())
        .is_none_or(|remaining| remaining.is_zero())
    {
        return None;
    }
    let mut command = Command::new("go");
    command.args(["env", variable]);
    let Ok(mut child) = command.stdout(std::process::Stdio::piped()).spawn() else {
        return None;
    };
    let status = wait_for_child_until(&mut child, deadline)?;
    if !status.success() {
        return None;
    }
    let mut output = String::new();
    child
        .stdout
        .take()
        .and_then(|mut stdout| std::io::Read::read_to_string(&mut stdout, &mut output).ok())
        .filter(|_| !output.trim().is_empty())?;
    Some(output.trim().to_owned())
}

fn push_unique_dir(dirs: &mut Vec<PathBuf>, candidate: Option<PathBuf>) {
    let Some(candidate) = candidate.filter(|path| !path.as_os_str().is_empty()) else {
        return;
    };
    if !dirs.iter().any(|existing| existing == &candidate) {
        dirs.push(candidate);
    }
}

fn executable_names(program: &str) -> Vec<String> {
    #[cfg(windows)]
    {
        let mut names = vec![program.to_owned()];
        if Path::new(program).extension().is_none() {
            names.push(format!("{program}.exe"));
        }
        names
    }
    #[cfg(not(windows))]
    {
        vec![program.to_owned()]
    }
}

/// Convert an absolute filesystem path to a `file://` URI.
#[must_use]
pub fn path_to_uri(path: &Path) -> String {
    Url::from_file_path(path)
        .map(|url| url.to_string())
        .unwrap_or_else(|()| format!("file://{}", path.display()))
}

fn uri_to_stable_path(uri: &str) -> String {
    Url::parse(uri)
        .ok()
        .and_then(|url| url.to_file_path().ok())
        .map(|path| path.to_string_lossy().into_owned())
        .unwrap_or_else(|| uri.to_owned())
}

fn should_skip_workspace_dir(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| {
            matches!(
                name,
                ".git"
                    | ".hg"
                    | ".svn"
                    | "target"
                    | "node_modules"
                    | ".next"
                    | ".cortex"
                    | "dist"
                    | "build"
            )
        })
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
    fn retry_navigation_permits_a_final_quick_retry_for_retryable_errors() {
        let mut attempts = 0;
        let result = retry_navigation(
            Instant::now() + Duration::from_millis(40),
            || {
                attempts += 1;
                if attempts == 1 {
                    Err(crate::LspError::ServerError {
                        code: LSP_REQUEST_CANCELLED,
                    })
                } else {
                    Ok(vec!["ready"])
                }
            },
            |items| !items.is_empty(),
        )
        .expect("a final quick retry should run while budget remains");

        assert_eq!(result, vec!["ready"]);
        assert_eq!(attempts, 2);
    }

    #[test]
    fn retry_navigation_permits_a_final_quick_retry_for_non_ready_results() {
        let mut attempts = 0;
        let result = retry_navigation(
            Instant::now() + Duration::from_millis(40),
            || {
                attempts += 1;
                if attempts == 1 {
                    Ok(Vec::<&str>::new())
                } else {
                    Ok(vec!["ready"])
                }
            },
            |items| !items.is_empty(),
        )
        .expect("a final quick retry should run while budget remains");

        assert_eq!(result, vec!["ready"]);
        assert_eq!(attempts, 2);
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

    #[test]
    fn server_run_state_labels_match_status_contract_vocabulary() {
        assert_eq!(ServerRunState::Disabled.label(), "disabled");
        assert_eq!(ServerRunState::Available.label(), "available");
        assert_eq!(ServerRunState::Installing.label(), "installing");
        assert_eq!(ServerRunState::Running(1).label(), "running");
        assert_eq!(ServerRunState::Unavailable.label(), "unavailable");
        assert_eq!(ServerRunState::Crashed.label(), "crashed");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn runtime_status_accepts_matching_owned_process_identity() {
        let identity = LspProcessIdentity::capture(std::process::id())
            .expect("the current Linux process must have start ticks");
        let mut state = RuntimeServerState {
            path: None,
            servers: BTreeMap::new(),
        };
        state.register("fixture", Some(identity));

        assert_eq!(
            state.status_for("fixture"),
            RegisteredServer::Running(identity.pid())
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn runtime_registration_persists_pid_and_start_ticks() {
        let identity = LspProcessIdentity::capture(std::process::id())
            .expect("the current Linux process must have start ticks");
        let root = std::env::temp_dir().join(format!(
            "cortex-lsp-runtime-registration-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system clock")
                .as_nanos()
        ));
        let path = root.join("runtime.json");
        let mut state = RuntimeServerState {
            path: Some(path.clone()),
            servers: BTreeMap::new(),
        };

        state.register("fixture", Some(identity));

        let persisted: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).expect("persisted runtime record"))
                .expect("runtime state JSON");
        assert_eq!(persisted["fixture"]["pid"], identity.pid());
        assert_eq!(persisted["fixture"]["start_time"], identity.start_time());
        let _ = std::fs::remove_dir_all(root);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn runtime_status_removes_dead_or_reused_pid_records() {
        let identity = LspProcessIdentity::capture(std::process::id())
            .expect("the current Linux process must have start ticks");
        let mut state = RuntimeServerState {
            path: None,
            servers: BTreeMap::from([(
                "fixture".to_owned(),
                ManagedServer {
                    pid: identity.pid(),
                    start_time: Some(identity.start_time().saturating_add(1)),
                },
            )]),
        };

        assert_eq!(state.status_for("fixture"), RegisteredServer::Stale);
        assert!(!state.servers.contains_key("fixture"));

        state.servers.insert(
            "dead".to_owned(),
            ManagedServer {
                pid: u32::MAX,
                start_time: Some(identity.start_time()),
            },
        );
        assert_eq!(state.status_for("dead"), RegisteredServer::Stale);
        assert!(!state.servers.contains_key("dead"));
    }

    #[test]
    fn runtime_status_fails_closed_and_persists_removal_for_legacy_pid_records() {
        let root = std::env::temp_dir().join(format!(
            "cortex-lsp-runtime-state-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system clock")
                .as_nanos()
        ));
        let path = root.join("runtime.json");
        let mut state = RuntimeServerState {
            path: Some(path.clone()),
            servers: BTreeMap::from([(
                "legacy".to_owned(),
                ManagedServer {
                    pid: std::process::id(),
                    start_time: None,
                },
            )]),
        };
        persist_runtime_servers(&path, &state.servers);

        assert_eq!(state.status_for("legacy"), RegisteredServer::Stale);
        assert!(!state.servers.contains_key("legacy"));
        let persisted: BTreeMap<String, ManagedServer> =
            serde_json::from_slice(&std::fs::read(&path).expect("persisted stale removal"))
                .expect("runtime state JSON");
        assert!(persisted.is_empty());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn runtime_registration_skips_unverified_process_identity() {
        let root = std::env::temp_dir().join(format!(
            "cortex-lsp-runtime-unverified-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system clock")
                .as_nanos()
        ));
        let path = root.join("runtime.json");
        let mut state = RuntimeServerState {
            path: Some(path.clone()),
            servers: BTreeMap::new(),
        };

        state.register("fixture", None);

        assert!(state.servers.is_empty());
        assert!(
            !path.exists(),
            "a platform without a stable process token must not persist PID-only ownership"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn runtime_unregister_preserves_a_replacement_identity() {
        let root = std::env::temp_dir().join(format!(
            "cortex-lsp-runtime-unregister-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system clock")
                .as_nanos()
        ));
        let path = root.join("runtime.json");
        let original = LspProcessIdentity::from_parts(101, 1001);
        let replacement = LspProcessIdentity::from_parts(202, 2002);
        let mut state = RuntimeServerState {
            path: Some(path.clone()),
            servers: BTreeMap::new(),
        };

        state.register("fixture", Some(replacement));
        state.unregister("fixture", Some(original));

        let persisted: BTreeMap<String, ManagedServer> =
            serde_json::from_slice(&std::fs::read(&path).expect("persisted runtime state"))
                .expect("runtime state JSON");
        assert_eq!(
            persisted.get("fixture"),
            Some(&ManagedServer {
                pid: replacement.pid(),
                start_time: Some(replacement.start_time()),
            })
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn runtime_stale_cleanup_cannot_erase_concurrently_registered_replacement() {
        use std::sync::{Arc, Barrier};

        let root = std::env::temp_dir().join(format!(
            "cortex-lsp-runtime-race-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system clock")
                .as_nanos()
        ));
        let path = root.join("runtime.json");
        let replacement_identity = LspProcessIdentity::capture(std::process::id())
            .expect("the current Linux process must have start ticks");
        let stale = ManagedServer {
            pid: replacement_identity.pid(),
            start_time: Some(replacement_identity.start_time().saturating_add(1)),
        };
        let replacement = ManagedServer {
            pid: replacement_identity.pid(),
            start_time: Some(replacement_identity.start_time()),
        };
        let mut stale_cleaner = RuntimeServerState {
            path: Some(path.clone()),
            servers: BTreeMap::from([("fixture".to_owned(), stale)]),
        };
        persist_runtime_servers(&path, &stale_cleaner.servers);
        let barrier = Arc::new(Barrier::new(2));

        std::thread::scope(|scope| {
            let registrar_barrier = barrier.clone();
            let registrar_path = path.clone();
            scope.spawn(move || {
                let mut registrar = RuntimeServerState {
                    path: Some(registrar_path),
                    servers: BTreeMap::new(),
                };
                registrar_barrier.wait();
                registrar.register(
                    "fixture",
                    LspProcessIdentity::from_parts(
                        replacement.pid,
                        replacement.start_time.expect("replacement start time"),
                    )
                    .into(),
                );
                registrar_barrier.wait();
            });

            barrier.wait();
            barrier.wait();
            assert_eq!(
                stale_cleaner.status_for("fixture"),
                RegisteredServer::Running(replacement.pid),
                "cleanup must observe the concurrently registered live replacement"
            );
        });

        let persisted: BTreeMap<String, ManagedServer> =
            serde_json::from_slice(&std::fs::read(&path).expect("persisted runtime state"))
                .expect("runtime state JSON");
        assert_eq!(
            persisted.get("fixture").map(|server| server.pid),
            Some(replacement.pid)
        );
        assert_eq!(
            persisted
                .get("fixture")
                .and_then(|server| server.start_time),
            replacement.start_time
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn status_rows_include_install_hints_and_distinguish_available_from_crashed() {
        let root = std::env::temp_dir().join(format!("cortex-lsp-status-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&root);
        let existing_program = root.join("existing-lsp");
        std::fs::write(&existing_program, "#!/bin/sh\nexit 0\n").expect("write fake lsp program");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(&existing_program)
                .expect("fake lsp metadata")
                .permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&existing_program, perms).expect("chmod fake lsp");
        }

        let installable_language = "installable";
        let unavailable_language = "unavailable";
        let disabled_language = "disabled";
        let available_language = "available";
        let crashed_language = "crashed";
        let missing_program = format!("missing-lsp-{}-install", std::process::id());
        let missing_no_plan_program = format!("missing-lsp-{}-none", std::process::id());
        let config = LspConfigSet::from_servers(vec![
            LspServerConfig::new(installable_language, &missing_program)
                .with_extension("ins")
                .with_install(InstallPlan::Npm {
                    package: "example-lsp".to_owned(),
                }),
            LspServerConfig::new(unavailable_language, &missing_no_plan_program)
                .with_extension("none"),
            LspServerConfig::new(disabled_language, &missing_no_plan_program)
                .with_extension("off")
                .with_enabled(false),
            LspServerConfig::new(available_language, existing_program.to_string_lossy())
                .with_extension("avail"),
            LspServerConfig::new(crashed_language, existing_program.to_string_lossy())
                .with_extension("crash"),
        ]);

        let mut manager = FeedbackManager::new(&root, config);
        manager
            .servers
            .insert(crashed_language.to_owned(), ServerSlot::Broken);
        let rows = manager.statuses();

        let installable = rows
            .iter()
            .find(|row| row.language_id == installable_language)
            .expect("installable row");
        assert!(
            matches!(
                installable.state,
                ServerRunState::Installing | ServerRunState::Unavailable
            ),
            "installable language should be installable unless no-net is active: {installable:?}"
        );
        assert!(
            installable
                .install_hint
                .as_ref()
                .is_some_and(|hint| hint.contains("npm install -g")),
            "installable row should surface install guidance: {installable:?}"
        );

        let unavailable = rows
            .iter()
            .find(|row| row.language_id == unavailable_language)
            .expect("unavailable row");
        assert_eq!(unavailable.state, ServerRunState::Unavailable);
        assert!(unavailable.install_hint.is_none());

        let disabled = rows
            .iter()
            .find(|row| row.language_id == disabled_language)
            .expect("disabled row");
        assert_eq!(disabled.state, ServerRunState::Disabled);

        let available = rows
            .iter()
            .find(|row| row.language_id == available_language)
            .expect("available row");
        assert_eq!(available.state, ServerRunState::Available);

        let crashed = rows
            .iter()
            .find(|row| row.language_id == crashed_language)
            .expect("crashed row");
        assert_eq!(crashed.state, ServerRunState::Crashed);
    }
}
