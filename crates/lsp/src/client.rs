//! Synchronous stdio JSON-RPC client for Language Server Protocol diagnostics.

use std::collections::BTreeMap;
use std::io::BufReader;
use std::process::{Child, ChildStdin, Command, ExitStatus, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use serde_json::{Map, Value, json};

use crate::LspError;
use crate::diagnostics::{
    Diagnostic, DiagnosticStore, PUBLISH_DIAGNOSTICS_METHOD, Position, PublishDiagnosticsParams,
    WorkspaceDiagnosticEntry, document_diagnostics_from_value, workspace_diagnostics_from_value,
};
use crate::navigation::{
    self, CallHierarchyItem, DocumentSymbol, Hover, IncomingCall, Location, OutgoingCall,
    SymbolInformation,
};
use crate::protocol::{InitializeParams, InitializeResult, JSONRPC_VERSION};
use crate::transport::{self, DEFAULT_MAX_MESSAGE_BYTES};

/// Default time to wait for one JSON-RPC response.
pub const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
/// Default grace period for a clean child exit before force-kill.
pub const DEFAULT_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);
const SHUTDOWN_POLL_INTERVAL: Duration = Duration::from_millis(10);
const DIAGNOSTICS_IDLE_INTERVAL: Duration = Duration::from_millis(1200);
// A warmed/open document can receive an immediate clean publish for a didChange while the server
// is still computing semantic diagnostics for the same generation. Keep empty didChange results
// alive slightly longer so post-mutation feedback does not miss the delayed real error.
const EMPTY_DID_CHANGE_DIAGNOSTICS_IDLE_INTERVAL: Duration = Duration::from_millis(2500);
const DOCUMENT_DIAGNOSTIC_METHOD: &str = "textDocument/diagnostic";
const WORKSPACE_DIAGNOSTIC_METHOD: &str = "workspace/diagnostic";

/// Configuration describing how to launch one server child process.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LspServerCommand {
    program: String,
    args: Vec<String>,
    env: Vec<(String, String)>,
    request_timeout: Duration,
    shutdown_timeout: Duration,
    max_message_bytes: usize,
}

impl LspServerCommand {
    /// Create a command that runs `program` with defaults.
    pub fn new(program: impl Into<String>) -> Self {
        Self {
            program: program.into(),
            args: Vec::new(),
            env: Vec::new(),
            request_timeout: DEFAULT_REQUEST_TIMEOUT,
            shutdown_timeout: DEFAULT_SHUTDOWN_TIMEOUT,
            max_message_bytes: DEFAULT_MAX_MESSAGE_BYTES,
        }
    }

    /// Append one argument.
    #[must_use]
    pub fn with_arg(mut self, arg: impl Into<String>) -> Self {
        self.args.push(arg.into());
        self
    }

    /// Append several arguments.
    #[must_use]
    pub fn with_args<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.args.extend(args.into_iter().map(Into::into));
        self
    }

    /// Set an environment variable for the server child.
    #[must_use]
    pub fn with_env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.env.push((key.into(), value.into()));
        self
    }

    /// Override response timeout.
    #[must_use]
    pub const fn with_request_timeout(mut self, timeout: Duration) -> Self {
        self.request_timeout = timeout;
        self
    }

    /// Program name.
    #[must_use]
    pub fn program(&self) -> &str {
        &self.program
    }

    /// Arguments.
    #[must_use]
    pub fn args(&self) -> &[String] {
        &self.args
    }

    /// Environment overrides.
    #[must_use]
    pub fn env(&self) -> &[(String, String)] {
        &self.env
    }

    fn validate(&self) -> Result<(), LspError> {
        if self.program.trim().is_empty() || self.program.contains('\0') {
            return Err(LspError::InvalidCommand);
        }
        if self.args.iter().any(|arg| arg.contains('\0')) {
            return Err(LspError::InvalidCommand);
        }
        if self.env.iter().any(|(key, value)| {
            key.is_empty() || key.contains('\0') || key.contains('=') || value.contains('\0')
        }) {
            return Err(LspError::InvalidCommand);
        }
        if self.max_message_bytes == 0 {
            return Err(LspError::InvalidCommand);
        }
        Ok(())
    }
}

enum ReaderEvent {
    Message(Vec<u8>),
    Closed,
    Failed(LspError),
}

#[derive(Debug)]
struct AppliedDiagnostics {
    uri: String,
    version: Option<i64>,
    generation: u64,
    diagnostics: Vec<Diagnostic>,
}

enum NotificationOutcome {
    AppliedDiagnostics(AppliedDiagnostics),
    IgnoredStaleDiagnostics,
    Handled,
    NotNotification,
}

impl NotificationOutcome {
    const fn is_handled(&self) -> bool {
        !matches!(self, Self::NotNotification)
    }
}

/// Running language server over stdio.
#[derive(Debug)]
pub struct LspClient {
    child: Child,
    process_identity: LspProcessIdentity,
    stdin: Option<ChildStdin>,
    incoming: Receiver<ReaderEvent>,
    reader: Option<JoinHandle<()>>,
    next_id: i64,
    diagnostics: DiagnosticStore,
    diagnostic_generation: u64,
    minimum_diagnostic_versions: BTreeMap<String, i64>,
    supports_document_diagnostics_pull: bool,
    supports_workspace_diagnostics_pull: bool,
    request_timeout: Duration,
    shutdown_timeout: Duration,
}

/// Cortex-owned child identity that remains stable when a wrapper execs its server process.
///
/// Linux start ticks identify one process lifetime. A later process may reuse the numeric PID, but
/// cannot have the same start ticks, and an `exec` preserves those ticks.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct LspProcessIdentity {
    pid: u32,
    start_time: u64,
}

impl LspProcessIdentity {
    pub(crate) const fn from_parts(pid: u32, start_time: u64) -> Self {
        Self { pid, start_time }
    }

    pub(crate) fn capture(pid: u32) -> Option<Self> {
        #[cfg(target_os = "linux")]
        {
            let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
            // `comm` (field 2) is wrapped in parens and can contain spaces or ')'. Split after
            // the last ')' so the first remaining field is process state (field 3).
            let after_comm = stat.rsplit_once(')')?.1;
            let mut fields = after_comm.split_whitespace();
            if fields.next()? == "Z" {
                return None;
            }
            // Field 22 is the Linux start time in ticks since boot. Field 3 has already been
            // consumed, so start time is the 19th remaining item (zero-based index 18).
            let start_time = fields.nth(18)?.parse::<u64>().ok()?;
            Some(Self { pid, start_time })
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = pid;
            None
        }
    }

    pub(crate) const fn pid(self) -> u32 {
        self.pid
    }

    pub(crate) const fn start_time(self) -> u64 {
        self.start_time
    }

    pub(crate) fn is_live(self) -> bool {
        Self::capture(self.pid).is_some_and(|current| current.start_time == self.start_time)
    }
}

impl LspClient {
    /// Spawn the configured server. Call [`initialize`](Self::initialize) next.
    ///
    /// # Errors
    /// Returns a typed error when the command is invalid or the child cannot be spawned.
    pub fn spawn(config: &LspServerCommand) -> Result<Self, LspError> {
        config.validate()?;
        let mut command = Command::new(config.program());
        command
            .args(config.args())
            .envs(config.env.iter().map(|(key, value)| (key, value)))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());

        let mut child = command.spawn().map_err(|_| LspError::Spawn)?;
        let Some(process_identity) = LspProcessIdentity::capture(child.id()) else {
            force_kill(&mut child);
            return Err(LspError::Spawn);
        };
        let stdin = match child.stdin.take() {
            Some(stdin) => stdin,
            None => {
                force_kill(&mut child);
                return Err(LspError::Io);
            }
        };
        let stdout = match child.stdout.take() {
            Some(stdout) => stdout,
            None => {
                force_kill(&mut child);
                return Err(LspError::Io);
            }
        };

        let (sender, receiver) = mpsc::channel();
        let max_bytes = config.max_message_bytes;
        let reader = thread::spawn(move || reader_loop(stdout, max_bytes, &sender));

        Ok(Self {
            child,
            process_identity,
            stdin: Some(stdin),
            incoming: receiver,
            reader: Some(reader),
            next_id: 1,
            diagnostics: DiagnosticStore::new(),
            diagnostic_generation: 0,
            minimum_diagnostic_versions: BTreeMap::new(),
            supports_document_diagnostics_pull: false,
            supports_workspace_diagnostics_pull: false,
            request_timeout: config.request_timeout,
            shutdown_timeout: config.shutdown_timeout,
        })
    }

    /// Perform the `initialize` handshake request.
    ///
    /// # Errors
    /// Returns timeout, protocol, server, or I/O errors from the JSON-RPC exchange.
    pub fn initialize(&mut self, params: &InitializeParams) -> Result<InitializeResult, LspError> {
        let id = self.send_request("initialize", params.to_value())?;
        let result = match self.await_response(id) {
            Ok(result) => result,
            Err(error) => {
                self.force_reap();
                return Err(error);
            }
        };
        let parsed =
            InitializeResult::from_result_value(result).inspect_err(|_| self.force_reap())?;
        self.supports_document_diagnostics_pull =
            parsed.capabilities().supports_document_diagnostics();
        self.supports_workspace_diagnostics_pull =
            parsed.capabilities().supports_workspace_diagnostics();
        Ok(parsed)
    }

    /// Send the `initialized` notification.
    ///
    /// # Errors
    /// Returns an I/O error if the notification cannot be written.
    pub fn initialized(&mut self) -> Result<(), LspError> {
        self.send_notification("initialized", Value::Object(Map::new()))
    }

    /// Whether the server advertised `textDocument/diagnostic` support.
    #[must_use]
    pub const fn supports_document_diagnostics_pull(&self) -> bool {
        self.supports_document_diagnostics_pull
    }

    /// Whether the server advertised `workspace/diagnostic` support.
    #[must_use]
    pub const fn supports_workspace_diagnostics_pull(&self) -> bool {
        self.supports_workspace_diagnostics_pull
    }

    /// Send `textDocument/didOpen` with full text.
    ///
    /// # Errors
    /// Returns an I/O error if the notification cannot be written.
    pub fn did_open(
        &mut self,
        uri: &str,
        language_id: &str,
        version: i64,
        text: &str,
    ) -> Result<(), LspError> {
        self.send_notification(
            "textDocument/didOpen",
            json!({
                "textDocument": {
                    "uri": uri,
                    "languageId": language_id,
                    "version": version,
                    "text": text,
                }
            }),
        )
    }

    /// Send `textDocument/didChange` replacing the full document text.
    ///
    /// # Errors
    /// Returns an I/O error if the notification cannot be written.
    pub fn did_change_full(&mut self, uri: &str, version: i64, text: &str) -> Result<(), LspError> {
        self.send_notification(
            "textDocument/didChange",
            json!({
                "textDocument": {"uri": uri, "version": version},
                "contentChanges": [{"text": text}],
            }),
        )
    }

    /// Drain any buffered diagnostics notifications without blocking.
    ///
    /// # Errors
    /// Returns when the server closed or framing failed.
    pub fn drain_pending_diagnostics(&mut self) -> Result<usize, LspError> {
        let mut applied = 0;
        loop {
            match self.incoming.try_recv() {
                Ok(ReaderEvent::Message(body)) => {
                    if matches!(
                        self.try_apply_notification(&body)?,
                        NotificationOutcome::AppliedDiagnostics(_)
                    ) {
                        applied += 1;
                    }
                }
                Ok(ReaderEvent::Closed) => return Err(LspError::Closed),
                Ok(ReaderEvent::Failed(error)) => return Err(error),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => return Err(LspError::Closed),
            }
        }
        Ok(applied)
    }

    /// Wait up to `timeout` for diagnostics notifications.
    ///
    /// # Errors
    /// Returns when the server closed or framing failed.
    pub fn poll_diagnostics(&mut self, timeout: Duration) -> Result<usize, LspError> {
        let deadline = Instant::now() + timeout;
        let mut applied = 0;
        loop {
            let remaining = deadline
                .checked_duration_since(Instant::now())
                .unwrap_or_default();
            let wait = if applied == 0 {
                remaining
            } else {
                DIAGNOSTICS_IDLE_INTERVAL.min(remaining)
            };
            if wait.is_zero() {
                break;
            }
            match self.incoming.recv_timeout(wait) {
                Ok(ReaderEvent::Message(body)) => {
                    if matches!(
                        self.try_apply_notification(&body)?,
                        NotificationOutcome::AppliedDiagnostics(_)
                    ) {
                        applied += 1;
                    }
                }
                Ok(ReaderEvent::Closed) => return Err(LspError::Closed),
                Ok(ReaderEvent::Failed(error)) => return Err(error),
                Err(RecvTimeoutError::Timeout) => break,
                Err(RecvTimeoutError::Disconnected) => return Err(LspError::Closed),
            }
        }
        Ok(applied)
    }

    /// Current monotonic generation for applied `publishDiagnostics` notifications.
    #[must_use]
    pub const fn diagnostic_generation(&self) -> u64 {
        self.diagnostic_generation
    }

    /// Remember the minimum document version that may update diagnostics for `uri`.
    ///
    /// Versioned `publishDiagnostics` notifications older than this are stale relative to the
    /// latest didOpen/didChange and are ignored instead of overwriting the current file's feedback.
    pub fn set_minimum_diagnostic_version(&mut self, uri: &str, version: i64) {
        self.minimum_diagnostic_versions
            .insert(uri.to_owned(), version);
    }

    /// Wait for a fresh diagnostics publish for `uri` after a didOpen/didChange boundary.
    ///
    /// A versioned publish is fresh only when its version is at least `document_version`. A server
    /// that omits `publishDiagnostics.version` is correlated by the post-change generation boundary:
    /// only a target-URI publish applied after `baseline_generation` is returned. Timeout returns
    /// `Ok(None)` so callers can avoid surfacing stale diagnostics from an older mutation.
    pub fn poll_fresh_diagnostics(
        &mut self,
        uri: &str,
        document_version: i64,
        baseline_generation: u64,
        timeout: Duration,
    ) -> Result<Option<Vec<Diagnostic>>, LspError> {
        let deadline = Instant::now() + timeout;
        let mut latest_fresh: Option<Vec<Diagnostic>> = None;
        loop {
            let remaining = deadline
                .checked_duration_since(Instant::now())
                .unwrap_or_default();
            if remaining.is_zero() {
                return Ok(latest_fresh);
            }
            let wait = if let Some(latest) = latest_fresh.as_ref() {
                diagnostics_idle_interval(document_version, latest).min(remaining)
            } else {
                remaining
            };
            match self.incoming.recv_timeout(wait) {
                Ok(ReaderEvent::Message(body)) => {
                    if let NotificationOutcome::AppliedDiagnostics(event) =
                        self.try_apply_notification(&body)?
                    {
                        if fresh_diagnostics_for(&event, uri, document_version, baseline_generation)
                        {
                            latest_fresh = Some(event.diagnostics);
                        }
                    }
                }
                Ok(ReaderEvent::Closed) => return Err(LspError::Closed),
                Ok(ReaderEvent::Failed(error)) => return Err(error),
                Err(RecvTimeoutError::Timeout) => return Ok(latest_fresh),
                Err(RecvTimeoutError::Disconnected) => return Err(LspError::Closed),
            }
        }
    }

    /// Diagnostics captured so far.
    #[must_use]
    pub const fn diagnostics(&self) -> &DiagnosticStore {
        &self.diagnostics
    }

    /// Request pull diagnostics for one document.
    ///
    /// # Errors
    /// Returns a typed LSP error if the request cannot complete.
    pub fn document_diagnostics(&mut self, uri: &str) -> Result<Vec<Diagnostic>, LspError> {
        self.document_diagnostics_until(uri, Instant::now() + self.request_timeout)
    }

    /// Request pull diagnostics for one document before `deadline`.
    ///
    /// # Errors
    /// Returns a typed LSP error if the request cannot complete before the deadline.
    pub(crate) fn document_diagnostics_until(
        &mut self,
        uri: &str,
        deadline: Instant,
    ) -> Result<Vec<Diagnostic>, LspError> {
        let id = self.send_request(
            DOCUMENT_DIAGNOSTIC_METHOD,
            json!({
                "textDocument": {"uri": uri}
            }),
        )?;
        let result = self.await_response_until(id, deadline)?;
        Ok(document_diagnostics_from_value(&result))
    }

    /// Request workspace-level diagnostics before `deadline`.
    ///
    /// # Errors
    /// Returns a typed LSP error if the request cannot complete before the deadline.
    pub(crate) fn workspace_diagnostics_until(
        &mut self,
        deadline: Instant,
    ) -> Result<Vec<WorkspaceDiagnosticEntry>, LspError> {
        let id = self.send_request(
            WORKSPACE_DIAGNOSTIC_METHOD,
            json!({
                "identifier": "cortex",
                "previousResultIds": []
            }),
        )?;
        let result = self.await_response_until(id, deadline)?;
        Ok(workspace_diagnostics_from_value(&result))
    }

    /// Child PID.
    #[must_use]
    pub fn child_id(&self) -> u32 {
        self.child.id()
    }

    /// Stable identity of the exact child process Cortex spawned.
    #[must_use]
    pub(crate) const fn process_identity(&self) -> LspProcessIdentity {
        self.process_identity
    }

    /// Request hover information for the symbol at `position` in `uri`.
    ///
    /// # Errors
    /// Returns a typed LSP error if the request times out, the server closes, or the response is
    /// malformed.
    pub fn hover(&mut self, uri: &str, position: Position) -> Result<Option<Hover>, LspError> {
        let id = self.send_request(
            navigation::HOVER_METHOD,
            navigation::text_document_position_params(uri, position),
        )?;
        let result = self.await_response(id)?;
        Ok(navigation::hover_from_value(&result))
    }

    /// Request definition locations for the symbol at `position` in `uri`.
    ///
    /// # Errors
    /// Returns a typed LSP error if the request cannot complete.
    pub fn goto_definition(
        &mut self,
        uri: &str,
        position: Position,
    ) -> Result<Vec<Location>, LspError> {
        let id = self.send_request(
            navigation::DEFINITION_METHOD,
            navigation::text_document_position_params(uri, position),
        )?;
        let result = self.await_response(id)?;
        Ok(navigation::locations_from_value(&result))
    }

    /// Request reference locations for the symbol at `position` in `uri`.
    ///
    /// # Errors
    /// Returns a typed LSP error if the request cannot complete.
    pub fn references(
        &mut self,
        uri: &str,
        position: Position,
        include_declaration: bool,
    ) -> Result<Vec<Location>, LspError> {
        let id = self.send_request(
            navigation::REFERENCES_METHOD,
            navigation::references_params(uri, position, include_declaration),
        )?;
        let result = self.await_response(id)?;
        Ok(navigation::locations_from_value(&result))
    }

    /// Request implementation locations for the symbol at `position` in `uri`.
    ///
    /// # Errors
    /// Returns a typed LSP error if the request cannot complete.
    pub fn goto_implementation(
        &mut self,
        uri: &str,
        position: Position,
    ) -> Result<Vec<Location>, LspError> {
        let id = self.send_request(
            navigation::IMPLEMENTATION_METHOD,
            navigation::text_document_position_params(uri, position),
        )?;
        let result = self.await_response(id)?;
        Ok(navigation::locations_from_value(&result))
    }

    /// Request prepared call hierarchy items for the symbol at `position` in `uri`.
    ///
    /// # Errors
    /// Returns a typed LSP error if the request cannot complete.
    pub fn prepare_call_hierarchy(
        &mut self,
        uri: &str,
        position: Position,
    ) -> Result<Vec<CallHierarchyItem>, LspError> {
        let id = self.send_request(
            navigation::PREPARE_CALL_HIERARCHY_METHOD,
            navigation::call_hierarchy_prepare_params(uri, position),
        )?;
        let result = self.await_response(id)?;
        Ok(navigation::call_hierarchy_items_from_value(&result))
    }

    /// Request incoming call hierarchy edges for the symbol at `position` in `uri`.
    ///
    /// # Errors
    /// Returns a typed LSP error if the request cannot complete.
    pub fn incoming_calls(
        &mut self,
        uri: &str,
        position: Position,
    ) -> Result<Vec<IncomingCall>, LspError> {
        let prepare_id = self.send_request(
            navigation::PREPARE_CALL_HIERARCHY_METHOD,
            navigation::call_hierarchy_prepare_params(uri, position),
        )?;
        let prepare_result = self.await_response(prepare_id)?;
        let Some(item) = first_call_hierarchy_item(&prepare_result) else {
            return Ok(Vec::new());
        };
        let id = self.send_request(
            navigation::INCOMING_CALLS_METHOD,
            navigation::call_hierarchy_direction_params(item),
        )?;
        let result = self.await_response(id)?;
        Ok(navigation::incoming_calls_from_value(&result))
    }

    /// Request outgoing call hierarchy edges for the symbol at `position` in `uri`.
    ///
    /// # Errors
    /// Returns a typed LSP error if the request cannot complete.
    pub fn outgoing_calls(
        &mut self,
        uri: &str,
        position: Position,
    ) -> Result<Vec<OutgoingCall>, LspError> {
        let prepare_id = self.send_request(
            navigation::PREPARE_CALL_HIERARCHY_METHOD,
            navigation::call_hierarchy_prepare_params(uri, position),
        )?;
        let prepare_result = self.await_response(prepare_id)?;
        let Some(item) = first_call_hierarchy_item(&prepare_result) else {
            return Ok(Vec::new());
        };
        let id = self.send_request(
            navigation::OUTGOING_CALLS_METHOD,
            navigation::call_hierarchy_direction_params(item),
        )?;
        let result = self.await_response(id)?;
        Ok(navigation::outgoing_calls_from_value(&result))
    }

    /// Request document symbols for `uri`.
    ///
    /// # Errors
    /// Returns a typed LSP error if the request cannot complete.
    pub fn document_symbols(&mut self, uri: &str) -> Result<Vec<DocumentSymbol>, LspError> {
        let id = self.send_request(
            navigation::DOCUMENT_SYMBOL_METHOD,
            navigation::document_symbol_params(uri),
        )?;
        let result = self.await_response(id)?;
        Ok(navigation::document_symbols_from_value(&result))
    }

    /// Request workspace symbols matching `query`.
    ///
    /// # Errors
    /// Returns a typed LSP error if the request cannot complete.
    pub fn workspace_symbols(&mut self, query: &str) -> Result<Vec<SymbolInformation>, LspError> {
        let id = self.send_request(
            navigation::WORKSPACE_SYMBOL_METHOD,
            navigation::workspace_symbol_params(query),
        )?;
        let result = self.await_response(id)?;
        Ok(navigation::workspace_symbols_from_value(&result))
    }

    /// Whether the child is still running.
    pub fn is_running(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    /// Send shutdown and await its response.
    ///
    /// # Errors
    /// Returns response errors from the server exchange.
    pub fn shutdown(&mut self) -> Result<(), LspError> {
        let id = self.send_request("shutdown", Value::Null)?;
        match self.await_response(id) {
            Ok(_) => Ok(()),
            Err(error) => {
                self.force_reap();
                Err(error)
            }
        }
    }

    /// Send `exit` best-effort and reap the child by exact process handle.
    ///
    /// # Errors
    /// Returns an I/O error if waiting or killing fails.
    pub fn exit(&mut self) -> Result<ExitStatus, LspError> {
        let _ = self.send_notification("exit", Value::Null);
        self.reap_child()
    }

    fn send_request(&mut self, method: &str, params: Value) -> Result<i64, LspError> {
        let id = self.next_id;
        self.next_id += 1;
        self.send(json!({
            "jsonrpc": JSONRPC_VERSION,
            "id": id,
            "method": method,
            "params": params,
        }))?;
        Ok(id)
    }

    fn send_notification(&mut self, method: &str, params: Value) -> Result<(), LspError> {
        self.send(json!({
            "jsonrpc": JSONRPC_VERSION,
            "method": method,
            "params": params,
        }))
    }

    fn send(&mut self, message: Value) -> Result<(), LspError> {
        let Some(stdin) = &mut self.stdin else {
            return Err(LspError::Closed);
        };
        let body = serde_json::to_vec(&message).map_err(|_| LspError::Protocol)?;
        transport::write_message(stdin, &body).map_err(|_| LspError::Io)
    }

    fn await_response(&mut self, id: i64) -> Result<Value, LspError> {
        self.await_response_until(id, Instant::now() + self.request_timeout)
    }

    fn await_response_until(&mut self, id: i64, deadline: Instant) -> Result<Value, LspError> {
        loop {
            let remaining = deadline
                .checked_duration_since(Instant::now())
                .ok_or(LspError::Timeout)?;
            let event = self
                .incoming
                .recv_timeout(remaining)
                .map_err(|err| match err {
                    RecvTimeoutError::Timeout => LspError::Timeout,
                    RecvTimeoutError::Disconnected => LspError::Closed,
                })?;
            match event {
                ReaderEvent::Message(body) => {
                    if let Some(result) = self.try_apply_response(id, &body)? {
                        return result;
                    }
                }
                ReaderEvent::Closed => return Err(LspError::Closed),
                ReaderEvent::Failed(error) => return Err(error),
            }
        }
    }

    fn try_apply_response(
        &mut self,
        id: i64,
        body: &[u8],
    ) -> Result<Option<Result<Value, LspError>>, LspError> {
        let message: Value = serde_json::from_slice(body).map_err(|_| LspError::Protocol)?;
        if self.try_apply_notification_value(&message)?.is_handled() {
            return Ok(None);
        }
        if message.get("id").and_then(Value::as_i64) != Some(id) {
            return Ok(None);
        }
        if let Some(error) = message.get("error") {
            let code = error.get("code").and_then(Value::as_i64).unwrap_or(0);
            return Ok(Some(Err(LspError::ServerError { code })));
        }
        Ok(Some(Ok(message
            .get("result")
            .cloned()
            .unwrap_or(Value::Null))))
    }

    fn try_apply_notification(&mut self, body: &[u8]) -> Result<NotificationOutcome, LspError> {
        let message: Value = serde_json::from_slice(body).map_err(|_| LspError::Protocol)?;
        self.try_apply_notification_value(&message)
    }

    fn try_apply_notification_value(
        &mut self,
        message: &Value,
    ) -> Result<NotificationOutcome, LspError> {
        if message.get("method").and_then(Value::as_str) == Some(PUBLISH_DIAGNOSTICS_METHOD) {
            let params = message
                .get("params")
                .and_then(PublishDiagnosticsParams::from_value)
                .ok_or(LspError::Protocol)?;
            return Ok(self.apply_diagnostics(params));
        }
        // Some servers (notably rust-analyzer) send workspace/diagnostic refresh requests to the
        // client. Acknowledge them so the server keeps making progress, but diagnostics are still
        // collected from publishDiagnostics notifications.
        if message.get("id").is_some() && message.get("method").is_some() {
            if let Some(id) = message.get("id").cloned() {
                let _ =
                    self.send(json!({"jsonrpc": JSONRPC_VERSION, "id": id, "result": Value::Null}));
            }
            return Ok(NotificationOutcome::Handled);
        }
        Ok(NotificationOutcome::NotNotification)
    }

    fn apply_diagnostics(&mut self, params: PublishDiagnosticsParams) -> NotificationOutcome {
        if let (Some(version), Some(minimum)) = (
            params.version,
            self.minimum_diagnostic_versions.get(&params.uri).copied(),
        ) {
            if version < minimum {
                return NotificationOutcome::IgnoredStaleDiagnostics;
            }
        }

        self.diagnostic_generation = self.diagnostic_generation.saturating_add(1);
        let generation = self.diagnostic_generation;
        let event = AppliedDiagnostics {
            uri: params.uri.clone(),
            version: params.version,
            generation,
            diagnostics: params.diagnostics.clone(),
        };
        self.diagnostics.apply(params);
        NotificationOutcome::AppliedDiagnostics(event)
    }

    fn reap_child(&mut self) -> Result<ExitStatus, LspError> {
        self.stdin.take();
        let deadline = Instant::now() + self.shutdown_timeout;
        loop {
            match self.child.try_wait().map_err(|_| LspError::Io)? {
                Some(status) => {
                    self.join_reader();
                    return Ok(status);
                }
                None if Instant::now() >= deadline => {
                    self.child.kill().map_err(|_| LspError::Io)?;
                    let status = self.child.wait().map_err(|_| LspError::Io)?;
                    self.join_reader();
                    return Ok(status);
                }
                None => thread::sleep(SHUTDOWN_POLL_INTERVAL),
            }
        }
    }

    pub(crate) fn force_reap(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        self.stdin.take();
        self.join_reader();
    }

    fn join_reader(&mut self) {
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}

impl Drop for LspClient {
    fn drop(&mut self) {
        let _ = self.exit();
    }
}

fn diagnostics_idle_interval(document_version: i64, diagnostics: &[Diagnostic]) -> Duration {
    if document_version > 1 && diagnostics.is_empty() {
        EMPTY_DID_CHANGE_DIAGNOSTICS_IDLE_INTERVAL
    } else {
        DIAGNOSTICS_IDLE_INTERVAL
    }
}

fn fresh_diagnostics_for(
    event: &AppliedDiagnostics,
    uri: &str,
    document_version: i64,
    baseline_generation: u64,
) -> bool {
    if event.uri != uri || event.generation <= baseline_generation {
        return false;
    }
    match event.version {
        Some(version) => version >= document_version,
        None => true,
    }
}

fn reader_loop(stdout: std::process::ChildStdout, max_bytes: usize, sender: &Sender<ReaderEvent>) {
    let mut reader = BufReader::new(stdout);
    loop {
        match transport::read_message(&mut reader, max_bytes) {
            Ok(body) => {
                if sender.send(ReaderEvent::Message(body)).is_err() {
                    break;
                }
            }
            Err(transport::FramingError::Eof) => {
                let _ = sender.send(ReaderEvent::Closed);
                break;
            }
            Err(error) => {
                let _ = sender.send(ReaderEvent::Failed(error.into()));
                break;
            }
        }
    }
}

fn force_kill(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

fn first_call_hierarchy_item(value: &Value) -> Option<Value> {
    match value {
        Value::Array(items) => items.first().cloned(),
        Value::Object(_) => Some(value.clone()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_empty_command_is_rejected() {
        assert_eq!(
            LspClient::spawn(&LspServerCommand::new("")).expect_err("empty command should fail"),
            LspError::InvalidCommand
        );
    }

    #[test]
    fn fresh_diagnostics_require_current_version_when_present() {
        let event = AppliedDiagnostics {
            uri: "file:///work/main.rs".to_owned(),
            version: Some(2),
            generation: 8,
            diagnostics: Vec::new(),
        };
        assert!(!fresh_diagnostics_for(&event, "file:///work/main.rs", 3, 7));
        assert!(fresh_diagnostics_for(&event, "file:///work/main.rs", 2, 7));
        assert!(!fresh_diagnostics_for(&event, "file:///work/main.rs", 2, 8));
    }

    #[test]
    fn unversioned_diagnostics_use_generation_boundary() {
        let event = AppliedDiagnostics {
            uri: "file:///work/main.rs".to_owned(),
            version: None,
            generation: 9,
            diagnostics: Vec::new(),
        };
        assert!(fresh_diagnostics_for(&event, "file:///work/main.rs", 99, 8));
        assert!(!fresh_diagnostics_for(
            &event,
            "file:///work/main.rs",
            99,
            9
        ));
        assert!(!fresh_diagnostics_for(
            &event,
            "file:///work/other.rs",
            99,
            8
        ));
    }
}
