//! The mid-session `/connect` provider-auth flow.
//!
//! Opened by the `/connect` slash command (not boot-only), this full-screen overlay reuses the
//! `auth_login.png` mono-orange language: the shared `✦ Cortex Code · connect` chrome, a centered
//! titled panel, an arrow-navigable selection bar, and the same MASKED key-input widget
//! ([`crate::auth_login::masked_input_box`]). Unlike the boot auth screen its provider list is
//! GENERALIZED to the full provider catalogue ([`providers::connectable_providers`], every profile
//! EXCEPT the excluded Zen provider) and it is a multi-step flow:
//!
//! 1. **Provider** — pick a provider to connect.
//! 2. **Method** — its auth methods: API KEY always, plus OAuth ONLY where the provider advertises
//!    it. OAuth drives the REAL [`mcp::oauth`] engine (loopback callback in the reserved 3200-3299
//!    range, CSRF `state`, code→token exchange) on a background runtime, mirroring the CLI bridge in
//!    `crates/cli/src/auth_cmd.rs`; a structured `oauth` credential is stored to `auth.json` (0600).
//! 3. **API key** — a MASKED entry (never echoed); confirming stores it via
//!    [`providers::store_credential`] to `auth.json` (0600) and RE-RESOLVES so the provider reads as
//!    connected immediately (usable in `/models` with no restart).
//! 4. **OAuth waiting** — an in-progress "waiting for authorization" state showing the URL to open
//!    while the loopback callback listener awaits the authorization response.
//!
//! Security invariant: neither the typed key nor an OAuth token is ever rendered or logged — the
//! masked input box shows one `•` per typed character, and both paths write straight to the 0600
//! credential store without echoing the secret (the authorization URL carries only the client id,
//! loopback redirect, and CSRF state — no secret). Failures render as the inline `✗ error` block
//! inside the panel (the VAL-ERR error surface), never a toast/splatter.

use std::path::{Path, PathBuf};
use std::sync::mpsc::Receiver;
use std::thread;
use std::time::Duration;

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::widgets::Paragraph;

use providers::{
    AuthPathInputs, OAuthEndpoints, connectable_providers, connected_providers,
    list_stored_providers, oauth_endpoints, remove_stored_credential, store_credential,
};
use redaction::guard_outbound;
use tui_components::style::ThemeColors;

use crate::app::App;
use crate::auth_login::masked_input_box;
use crate::glyphs;
use crate::ui::{cells_to_line, place_segments};
use crate::view;

/// How long the OAuth flow waits for the loopback callback before giving up (the CLI bridge uses the
/// same 3-minute-class budget for an interactive authorization).
const OAUTH_TIMEOUT: Duration = Duration::from_secs(180);

/// The breadcrumb crumb shown after `✦ Cortex Code ·` on the connect screen.
const CRUMB: &str = "connect";
/// The amber title embedded in the panel's top border.
const PANEL_TITLE: &str = "Connect a provider";
/// Width (columns) of the centered panel.
const PANEL_W: u16 = 68;
/// Height (rows) of the centered panel (tall enough to list the provider catalogue at 92×30).
const PANEL_H: u16 = 24;

/// The amber `▎` left edge of the selected row.
const EDGE_COL: usize = 0;
/// The `›` selection marker of the selected row.
const MARKER_COL: usize = 1;
/// The row's main label column (provider name / method label).
const NAME_COL: usize = 3;
/// The right-hand status/meta column for a row.
const META_COL: usize = 26;
/// Left content column for subtitles, prompts, the input box, the error line, and hints.
const CONTENT_COL: usize = 2;

/// The current step of the connect flow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// Pick a provider to connect.
    Provider,
    /// Pick an auth method for the chosen provider.
    Method,
    /// Enter a masked API key for the chosen provider.
    ApiKey,
    /// The OAuth flow is running: waiting for the browser authorization + loopback callback.
    OAuthWaiting,
    /// The provider has been connected (success confirmation).
    Connected,
    /// Confirm removing a connected provider's stored credential (confirm-before-remove).
    ConfirmDisconnect,
    /// The provider has been disconnected (its credential removed) — confirmation note.
    Disconnected,
}

/// An auth method offered for a provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Method {
    /// A pasted API key (masked input). Offered for every provider.
    ApiKey,
    /// An OAuth authorization flow. Offered only where the provider advertises OAuth.
    OAuth,
}

impl Method {
    /// The method's row label.
    const fn label(self) -> &'static str {
        match self {
            Method::ApiKey => "API key",
            Method::OAuth => "OAuth",
        }
    }

    /// The faint per-row description.
    const fn desc(self) -> &'static str {
        match self {
            Method::ApiKey => "paste a provider API key",
            Method::OAuth => "authorize in your browser",
        }
    }
}

/// A state update published by the background OAuth worker for the UI thread to apply.
enum OAuthMessage {
    /// The loopback listener is up and the authorization URL is ready; the TUI shows the
    /// "waiting for authorization" state with the URL to open. Carries no secret.
    AwaitingAuthorization {
        /// The full authorization URL (client id + loopback redirect + CSRF state, no secret).
        authorize_url: String,
        /// The loopback redirect URI the callback listens on (in the reserved 3200-3299 range).
        redirect_uri: String,
    },
    /// A valid callback was exchanged and a structured `oauth` credential was stored (0600).
    Connected,
    /// The flow failed (bad/absent CSRF state, a denial, a timeout, or a store error). The message
    /// is secret-free and renders as the inline `✗ error` block.
    Failed(String),
}

/// The live OAuth handshake state held while the flow is running: the channel the background worker
/// publishes on, the authorization URL / loopback redirect once the listener is up, and the auth
/// store the worker writes to (so the connected state re-resolves from the same store on success).
struct OAuthProgress {
    /// The receiver drained each tick by [`ConnectState::poll_oauth`].
    rx: Receiver<OAuthMessage>,
    /// The authorization URL to open, once the worker reports the listener is up.
    authorize_url: Option<String>,
    /// The loopback redirect URI the callback listens on, once known.
    redirect_uri: Option<String>,
    /// The 0600 auth store the worker persists the credential to.
    auth_path: PathBuf,
}

/// Run the real [`mcp::oauth`] authorization-code flow on a dedicated background thread with its own
/// current-thread Tokio runtime (mirroring `crates/cli/src/auth_cmd.rs::authorize_builtin_oauth_method`
/// and the TUI turn worker), publishing state back over a channel the UI drains each tick.
///
/// The worker: binds the loopback callback listener + builds the authorization URL ([`begin`]),
/// reports [`OAuthMessage::AwaitingAuthorization`], waits for a valid callback (matching CSRF state)
/// and exchanges the code at the token endpoint ([`AuthorizationRequest::complete`]), then stores a
/// STRUCTURED `oauth` credential (access/refresh tokens, expiry, token endpoint, client id) to the
/// 0600 auth store. A mismatched/absent state is rejected by the engine and stores NOTHING; any
/// failure is reported as [`OAuthMessage::Failed`] with a secret-free message.
///
/// [`begin`]: mcp::oauth::begin
/// [`AuthorizationRequest::complete`]: mcp::oauth::AuthorizationRequest::complete
fn spawn_oauth(
    config: mcp::oauth::OAuthConfig,
    callback_port: Option<u16>,
    timeout: Duration,
    auth_path: PathBuf,
    provider_id: String,
) -> Receiver<OAuthMessage> {
    let (tx, rx) = std::sync::mpsc::channel();
    thread::spawn(move || {
        let runtime = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(runtime) => runtime,
            Err(err) => {
                let _ = tx.send(OAuthMessage::Failed(format!(
                    "failed to start OAuth runtime: {err}"
                )));
                return;
            }
        };
        let request = match runtime.block_on(mcp::oauth::begin(config, callback_port)) {
            Ok(request) => request,
            Err(err) => {
                let _ = tx.send(OAuthMessage::Failed(err.to_string()));
                return;
            }
        };
        // The authorize URL + loopback redirect carry no secret; surfacing them is the "open this
        // URL / waiting for authorization" state.
        if tx
            .send(OAuthMessage::AwaitingAuthorization {
                authorize_url: request.authorize_url.clone(),
                redirect_uri: request.redirect_uri.clone(),
            })
            .is_err()
        {
            // The flow was cancelled before the listener came up; nothing to wait for.
            return;
        }
        match runtime.block_on(request.complete(timeout)) {
            Ok(token) => {
                // Persist a STRUCTURED oauth credential (type oauth, access/refresh tokens, expiry,
                // token endpoint, client id) so runtime_auth.rs can unwrap/refresh it. The token is
                // written straight to the 0600 store and never echoed.
                let credential = oauth_credential_json(&token);
                match store_credential(&auth_path, &provider_id, &credential) {
                    Ok(()) => {
                        let _ = tx.send(OAuthMessage::Connected);
                    }
                    Err(err) => {
                        // `io::Error` carries no token material, so this message is secret-free.
                        let _ = tx.send(OAuthMessage::Failed(format!(
                            "Could not store credential: {err}"
                        )));
                    }
                }
            }
            Err(err) => {
                // A mismatched/absent CSRF state, a denial, or a timeout — the engine stored
                // nothing; surface a secret-free inline error.
                let _ = tx.send(OAuthMessage::Failed(err.to_string()));
            }
        }
    });
    rx
}

/// Serialize a freshly-exchanged OAuth token into the STRUCTURED `oauth` credential JSON persisted
/// to the 0600 auth store (mirroring the CLI bridge). The shape matches what
/// `providers::runtime_auth` reads back (`OAuthCredential::from_stored`) so the credential unwraps
/// and refreshes on expiry.
fn oauth_credential_json(token: &mcp::StoredToken) -> String {
    let mut credential = serde_json::Map::new();
    credential.insert(
        "type".to_owned(),
        serde_json::Value::String("oauth".to_owned()),
    );
    credential.insert(
        "access_token".to_owned(),
        serde_json::Value::String(token.access_token.clone()),
    );
    credential.insert(
        "token_type".to_owned(),
        serde_json::Value::String(token.token_type.clone()),
    );
    if let Some(refresh_token) = &token.refresh_token {
        credential.insert(
            "refresh_token".to_owned(),
            serde_json::Value::String(refresh_token.clone()),
        );
    }
    if let Some(expires_at) = token.expires_at {
        credential.insert(
            "expires_at".to_owned(),
            serde_json::Value::Number(expires_at.into()),
        );
    }
    credential.insert(
        "token_endpoint".to_owned(),
        serde_json::Value::String(token.token_endpoint.clone()),
    );
    credential.insert(
        "client_id".to_owned(),
        serde_json::Value::String(token.client_id.clone()),
    );
    serde_json::to_string(&serde_json::Value::Object(credential)).unwrap_or_default()
}

/// One connectable provider row: its stable id (credential-store key), display name, whether it
/// advertises OAuth, and its current connected (stored-credential) state.
#[derive(Debug, Clone)]
struct ProviderRow {
    /// The provider id (credential-store key).
    id: &'static str,
    /// The provider's human display name.
    display: &'static str,
    /// Whether the provider offers an OAuth method (API key is always offered).
    supports_oauth: bool,
    /// Whether the provider currently has a stored credential.
    connected: bool,
}

/// The connect-flow state: the generalized provider list, the current step and selections, the
/// (never-rendered) typed key buffer, and an optional inline error/notice.
pub struct ConnectState {
    /// The connectable providers (all profiles minus the excluded Zen provider).
    providers: Vec<ProviderRow>,
    /// The highlighted provider row.
    provider_selected: usize,
    /// The current step.
    step: Step,
    /// The auth methods offered for the chosen provider (built when entering the method step).
    methods: Vec<Method>,
    /// The highlighted method row.
    method_selected: usize,
    /// The raw typed API key. Held only to persist it to the 0600 store; never rendered/logged.
    key_input: String,
    /// An inline error rendered as the `✗ error` block inside the panel (never a toast).
    error: Option<String>,
    /// A faint inline notice.
    notice: Option<String>,
    /// The live OAuth handshake state while the OAuth flow is running (its background worker channel
    /// plus the authorization URL / loopback redirect once the listener is up).
    oauth: Option<OAuthProgress>,
}

impl ConnectState {
    /// Build the connect-flow state from the connectable provider catalogue (all profiles minus the
    /// excluded Zen provider), resolving each provider's current connected state, with the first
    /// provider highlighted on the provider step.
    #[must_use]
    pub fn new() -> Self {
        let connected = connected_providers();
        let providers = connectable_providers()
            .into_iter()
            .map(|provider| ProviderRow {
                id: provider.id,
                display: provider.display_name,
                supports_oauth: provider.supports_oauth,
                connected: connected.iter().any(|id| id == provider.id),
            })
            .collect();
        Self {
            providers,
            provider_selected: 0,
            step: Step::Provider,
            methods: Vec::new(),
            method_selected: 0,
            key_input: String::new(),
            error: None,
            notice: None,
            oauth: None,
        }
    }

    /// The highlighted provider row (clamped defensively).
    fn selected_provider(&self) -> &ProviderRow {
        let idx = self.provider_selected.min(self.providers.len().saturating_sub(1));
        &self.providers[idx]
    }

    /// The current step (drives the app's modal key routing).
    #[must_use]
    pub fn step(&self) -> Step {
        self.step
    }

    /// Move the highlighted row by `delta` (wrapping) within the active step's list.
    pub fn move_selection(&mut self, delta: i32) {
        match self.step {
            Step::Provider if !self.providers.is_empty() => {
                let n = self.providers.len() as i32;
                self.provider_selected = (self.provider_selected as i32 + delta).rem_euclid(n) as usize;
            }
            Step::Method if !self.methods.is_empty() => {
                let n = self.methods.len() as i32;
                self.method_selected = (self.method_selected as i32 + delta).rem_euclid(n) as usize;
            }
            _ => {}
        }
    }

    /// Advance from the provider step to the method step, building the chosen provider's method set:
    /// API key always, plus OAuth only where the provider advertises it.
    pub fn enter_provider(&mut self) {
        let mut methods = vec![Method::ApiKey];
        if self.selected_provider().supports_oauth {
            methods.push(Method::OAuth);
        }
        self.methods = methods;
        self.method_selected = 0;
        self.error = None;
        self.notice = None;
        self.step = Step::Method;
    }

    /// Act on the highlighted method: API key opens the masked entry; OAuth starts the real
    /// [`mcp::oauth`] handshake on a background runtime and moves to the waiting step.
    pub fn enter_method(&mut self) {
        match self.methods.get(self.method_selected).copied() {
            Some(Method::ApiKey) => {
                self.key_input.clear();
                self.error = None;
                self.notice = None;
                self.step = Step::ApiKey;
            }
            Some(Method::OAuth) => self.start_oauth(),
            None => {}
        }
    }

    /// Start the OAuth flow for the highlighted provider: fail closed on the network kill-switch,
    /// resolve the provider's OAuth endpoints (env-overridable for a controlled loopback endpoint)
    /// and the 0600 auth-store path from the process, then spawn the background handshake worker.
    /// An unavailable store, a provider without OAuth config, or a blocked network render an inline
    /// error and start nothing.
    fn start_oauth(&mut self) {
        let display = self.selected_provider().display.to_owned();
        let id = self.selected_provider().id.to_owned();
        // Fail closed on CORTEX_NO_NET before binding a listener or opening any socket.
        if let Err(diag) = guard_outbound("provider OAuth authorization") {
            self.error = Some(diag.to_string());
            return;
        }
        let Some(path) = AuthPathInputs::from_process().resolve() else {
            self.error = Some("No credential store is available.".to_owned());
            return;
        };
        let Some(endpoints) = oauth_endpoints(&id) else {
            self.error = Some(format!("OAuth is not configured for {display}."));
            return;
        };
        self.begin_oauth(endpoints, path, id);
    }

    /// Spawn the background OAuth worker for the resolved `endpoints`, storing to `path` under
    /// `provider_id`, and move to the waiting step. Separated from [`Self::start_oauth`] so a test
    /// can drive the handshake against a controlled loopback endpoint with explicit inputs.
    pub(crate) fn begin_oauth(
        &mut self,
        endpoints: OAuthEndpoints,
        path: PathBuf,
        provider_id: String,
    ) {
        let config = mcp::oauth::OAuthConfig {
            authorize_endpoint: endpoints.authorize_endpoint,
            token_endpoint: endpoints.token_endpoint,
            client_id: endpoints.client_id,
            scopes: endpoints.scopes,
        };
        let rx = spawn_oauth(
            config,
            endpoints.callback_port,
            OAUTH_TIMEOUT,
            path.clone(),
            provider_id,
        );
        self.oauth = Some(OAuthProgress {
            rx,
            authorize_url: None,
            redirect_uri: None,
            auth_path: path,
        });
        self.error = None;
        self.notice = None;
        self.step = Step::OAuthWaiting;
    }

    /// Drain the background OAuth worker's messages and apply them: record the authorization URL /
    /// loopback redirect once the listener is up, transition to the connected step (re-resolving the
    /// connected state) on success, or fall back to the method step with an inline error on failure.
    /// A no-op when no OAuth flow is running.
    pub fn poll_oauth(&mut self) {
        let mut drained = Vec::new();
        if let Some(progress) = self.oauth.as_ref() {
            while let Ok(message) = progress.rx.try_recv() {
                drained.push(message);
            }
        }
        for message in drained {
            match message {
                OAuthMessage::AwaitingAuthorization {
                    authorize_url,
                    redirect_uri,
                } => {
                    if let Some(progress) = self.oauth.as_mut() {
                        progress.authorize_url = Some(authorize_url);
                        progress.redirect_uri = Some(redirect_uri);
                    }
                }
                OAuthMessage::Connected => {
                    let auth_path = self
                        .oauth
                        .as_ref()
                        .map(|progress| progress.auth_path.clone());
                    self.oauth = None;
                    self.error = None;
                    self.notice = None;
                    if let Some(path) = auth_path {
                        self.refresh_connected_from(&path);
                    }
                    self.step = Step::Connected;
                }
                OAuthMessage::Failed(message) => {
                    self.oauth = None;
                    self.error = Some(message);
                    self.step = Step::Method;
                }
            }
        }
    }

    /// Cancel a running OAuth flow (dropping the worker channel) and return to the method step. The
    /// background listener winds down on its own timeout; nothing is stored.
    pub fn cancel_oauth(&mut self) {
        self.oauth = None;
        self.error = None;
        self.notice = None;
        self.step = Step::Method;
    }

    /// Return to the provider step from the method step.
    pub fn back_to_provider(&mut self) {
        self.error = None;
        self.notice = None;
        self.step = Step::Provider;
    }

    /// Request disconnecting the highlighted provider: if it currently has a stored credential, move
    /// to the confirm-before-remove step; otherwise it is a no-op (there is nothing to disconnect).
    pub fn request_disconnect(&mut self) {
        if self.selected_provider().connected {
            self.error = None;
            self.notice = None;
            self.step = Step::ConfirmDisconnect;
        }
    }

    /// Cancel a pending disconnect, returning to the provider step with nothing removed.
    pub fn cancel_disconnect(&mut self) {
        self.error = None;
        self.notice = None;
        self.step = Step::Provider;
    }

    /// Confirm the disconnect: remove the highlighted provider's stored credential from the process's
    /// 0600 auth store and, on success, re-resolve so it reads as not connected. A missing store or a
    /// removal error yields an inline error and removes nothing. Returns whether an entry was removed.
    #[must_use]
    pub fn confirm_disconnect(&mut self) -> bool {
        match AuthPathInputs::from_process().resolve() {
            Some(path) => self.disconnect_to(&path),
            None => {
                self.error = Some("No credential store is available.".to_owned());
                false
            }
        }
    }

    /// Remove the highlighted provider's stored credential from the auth store at `path`, then
    /// re-resolve the connected state from that store so it reflects immediately (and, on the next
    /// `/models` open, the provider reads as not connected). No secret is read or shown. Returns
    /// whether an entry was removed.
    pub(crate) fn disconnect_to(&mut self, path: &Path) -> bool {
        let id = self.selected_provider().id;
        match remove_stored_credential(path, id) {
            Ok(removed) => {
                self.error = None;
                self.notice = None;
                self.refresh_connected_from(path);
                self.step = Step::Disconnected;
                removed
            }
            Err(err) => {
                // `io::Error` carries no key material, so this message is secret-free.
                self.error = Some(format!("Could not remove credential: {err}"));
                false
            }
        }
    }

    /// Return to the method step from the API-key step, discarding any typed key.
    pub fn back_to_method(&mut self) {
        self.key_input.clear();
        self.error = None;
        self.step = Step::Method;
    }

    /// Append a typed character to the masked key (only on the API-key step).
    pub fn type_char(&mut self, c: char) {
        if self.step == Step::ApiKey && !c.is_control() {
            self.key_input.push(c);
            self.error = None;
        }
    }

    /// Remove the last typed character from the masked key (only on the API-key step).
    pub fn backspace(&mut self) {
        if self.step == Step::ApiKey {
            self.key_input.pop();
        }
    }

    /// Confirm the API-key entry: store the typed key to the process's 0600 credential store and, on
    /// success, re-resolve so the provider reads as connected. An empty key or a missing store
    /// yields an inline error and stores nothing. Returns whether a credential was stored.
    #[must_use]
    pub fn confirm_api_key(&mut self) -> bool {
        match AuthPathInputs::from_process().resolve() {
            Some(path) => self.confirm_api_key_to(&path),
            None => {
                self.error = Some("No credential store is available.".to_owned());
                false
            }
        }
    }

    /// Store the typed key for the highlighted provider into the 0600 auth store at `path`, then
    /// re-resolve the connected state from that store. The raw key is written straight to the store
    /// and never echoed. Returns whether a credential was stored.
    pub(crate) fn confirm_api_key_to(&mut self, path: &Path) -> bool {
        let key = self.key_input.trim();
        if key.is_empty() {
            self.error = Some("API key cannot be empty.".to_owned());
            return false;
        }
        let id = self.selected_provider().id;
        match store_credential(path, id, key) {
            Ok(()) => {
                self.key_input.clear();
                self.error = None;
                self.notice = None;
                self.refresh_connected_from(path);
                self.step = Step::Connected;
                true
            }
            Err(err) => {
                // `io::Error` carries no key material, so this message is secret-free.
                self.error = Some(format!("Could not store credential: {err}"));
                false
            }
        }
    }

    /// Re-resolve each provider's connected state from the auth store at `path` (so a just-stored
    /// provider reads as connected immediately).
    fn refresh_connected_from(&mut self, path: &Path) {
        let stored = list_stored_providers(path).unwrap_or_default();
        for provider in &mut self.providers {
            provider.connected = stored.iter().any(|id| id == provider.id);
        }
    }

    /// The number of typed key characters (drives the count of mask bullets); never the key itself.
    fn key_len(&self) -> usize {
        self.key_input.chars().count()
    }

    /// The authorization URL to open while the OAuth flow waits for the callback, once the loopback
    /// listener is up. Carries only the client id, loopback redirect, and CSRF state — no secret.
    fn oauth_authorize_url(&self) -> Option<&str> {
        self.oauth
            .as_ref()
            .and_then(|progress| progress.authorize_url.as_deref())
    }
}

impl Default for ConnectState {
    fn default() -> Self {
        Self::new()
    }
}

/// Render the mid-session `/connect` screen into `area`: the shared `✦ Cortex Code · connect` chrome
/// then the centered titled panel with the step-appropriate body (provider list / method list /
/// masked key entry / connected confirmation).
pub fn draw_connect(frame: &mut Frame, app: &App, area: Rect, theme: &ThemeColors) {
    let Some(state) = app.connect_state() else {
        return;
    };
    let body = view::render_chrome(frame, area, app, CRUMB, theme);
    if body.width == 0 || body.height == 0 {
        return;
    }
    let panel_w = PANEL_W.min(body.width);
    let panel_h = PANEL_H.min(body.height.saturating_sub(1));
    if panel_w < 8 || panel_h < 8 {
        return;
    }
    let panel_area = Rect {
        x: body.x + body.width.saturating_sub(panel_w) / 2,
        y: body.y + 1,
        width: panel_w,
        height: panel_h,
    };
    let inner = view::render_panel(frame, panel_area, PANEL_TITLE, None, theme);
    let lines = panel_body_lines(state, inner.width as usize, inner.height as usize, theme);
    frame.render_widget(
        Paragraph::new(lines).style(Style::default().bg(theme.background)),
        inner,
    );
}

/// Build the panel interior for the active step: a subtitle, the step's rows (a scrolled provider
/// list / the method list / the masked key box / the connected note), an optional inline error, and
/// a key-hints footer.
fn panel_body_lines(
    state: &ConnectState,
    width: usize,
    height: usize,
    theme: &ThemeColors,
) -> Vec<Line<'static>> {
    let base = Style::default().fg(theme.text).bg(theme.background);
    let faint = Style::default().fg(theme.text_muted).bg(theme.background);
    let text = Style::default().fg(theme.text).bg(theme.background);

    let mut lines: Vec<Line<'static>> = Vec::with_capacity(height.max(1));
    match state.step {
        Step::Provider => {
            lines.push(content_row(width, base, "select a provider to connect", faint));
            lines.push(row(width, base, &[]));
            // Window the provider list so the highlighted row stays visible within the panel.
            let capacity = height.saturating_sub(4).max(1);
            let (start, end) = window(state.provider_selected, state.providers.len(), capacity);
            for idx in start..end {
                lines.push(provider_row(state, idx, width, theme));
            }
        }
        Step::Method => {
            let display = state.selected_provider().display;
            lines.push(content_row(width, base, &format!("sign in to {display}"), faint));
            lines.push(row(width, base, &[]));
            for idx in 0..state.methods.len() {
                lines.push(method_row(state, idx, width, theme));
            }
            if let Some(notice) = &state.notice {
                lines.push(row(width, base, &[]));
                lines.push(content_row(width, base, notice, faint));
            }
        }
        Step::ApiKey => {
            let display = state.selected_provider().display;
            lines.push(content_row(
                width,
                base,
                &format!("paste API key for {display}:"),
                text,
            ));
            lines.extend(masked_input_box(state.key_len(), "", true, width, theme));
            lines.push(row(width, base, &[]));
            lines.push(stored_note_line(width, theme));
        }
        Step::OAuthWaiting => {
            let display = state.selected_provider().display;
            let amber = Style::default().fg(theme.secondary).bg(theme.background);
            lines.push(content_row(
                width,
                base,
                &format!("waiting for authorization — {display}"),
                text,
            ));
            lines.push(row(width, base, &[]));
            match state.oauth_authorize_url() {
                Some(url) => {
                    lines.push(content_row(width, base, "open this URL to authorize:", faint));
                    // The URL can exceed the panel width; wrap it so the whole (secret-free) URL is
                    // visible for the user to open.
                    let wrap_at = width.saturating_sub(CONTENT_COL).max(1);
                    for chunk in wrap_text(url, wrap_at) {
                        lines.push(content_row(width, base, &chunk, amber));
                    }
                    lines.push(row(width, base, &[]));
                    lines.push(content_row(
                        width,
                        base,
                        "cortex is listening on the loopback callback for the response",
                        faint,
                    ));
                }
                None => {
                    lines.push(content_row(
                        width,
                        base,
                        "starting the loopback callback listener…",
                        faint,
                    ));
                }
            }
        }
        Step::Connected => {
            let display = state.selected_provider().display;
            let okg = Style::default().fg(theme.success).bg(theme.background);
            lines.push(row(
                width,
                base,
                &[(
                    CONTENT_COL,
                    vec![(format!("{} connected {display}", glyphs::MARK_DONE), okg)],
                )],
            ));
            lines.push(row(width, base, &[]));
            lines.push(content_row(
                width,
                base,
                "it is usable now — open /models to pick one of its models",
                faint,
            ));
        }
        Step::ConfirmDisconnect => {
            let display = state.selected_provider().display;
            lines.push(content_row(
                width,
                base,
                &format!("Disconnect {display}?"),
                text,
            ));
            lines.push(row(width, base, &[]));
            lines.push(content_row(
                width,
                base,
                "this removes its stored credential from auth.json",
                faint,
            ));
            lines.push(content_row(
                width,
                base,
                "its models will require reconnecting to use",
                faint,
            ));
        }
        Step::Disconnected => {
            let display = state.selected_provider().display;
            let muted = Style::default().fg(theme.text_muted).bg(theme.background);
            let bright = Style::default().fg(theme.text_bright).bg(theme.background);
            lines.push(row(
                width,
                base,
                &[(
                    CONTENT_COL,
                    vec![
                        (format!("{} disconnected ", glyphs::DOT_HOLLOW), muted),
                        (display.to_owned(), bright),
                    ],
                )],
            ));
            lines.push(row(width, base, &[]));
            lines.push(content_row(
                width,
                base,
                "its models now require reconnecting to use",
                faint,
            ));
        }
    }

    // Inline error block (`✗ <message>`) in the ERR accent — never a toast/splatter.
    if let Some(message) = &state.error {
        lines.push(row(width, base, &[]));
        let err = Style::default().fg(theme.error).bg(theme.background);
        lines.push(row(
            width,
            base,
            &[(
                CONTENT_COL,
                vec![(format!("{} {message}", glyphs::MARK_ERROR), err)],
            )],
        ));
    }

    // Pad so the hints footer sits on the panel's last interior row.
    let hints = hints_line(state.step, width, theme);
    while lines.len() + 1 < height {
        lines.push(row(width, base, &[]));
    }
    lines.push(hints);
    lines
}

/// The visible `[start, end)` window of a list of `len` rows with `capacity` visible slots, scrolled
/// so `selected` stays visible (centered once past the first page).
fn window(selected: usize, len: usize, capacity: usize) -> (usize, usize) {
    if len <= capacity {
        return (0, len);
    }
    let half = capacity / 2;
    let start = selected.saturating_sub(half).min(len - capacity);
    (start, start + capacity)
}

/// Split `text` into chunks of at most `width` characters (char-based; authorization URLs are
/// ASCII), so a long URL wraps across panel rows rather than being clipped.
fn wrap_text(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let chars: Vec<char> = text.chars().collect();
    if chars.is_empty() {
        return vec![String::new()];
    }
    chars
        .chunks(width)
        .map(|chunk| chunk.iter().collect::<String>())
        .collect()
}

/// One provider row: the selected row is a strong inverse bar (solid ORANGE fill, near-black text,
/// `›` marker, amber `▎` edge); other rows show the name (BRIGHT) and a right-aligned connected
/// status (`✓ connected` OKG-green / `○ not connected` FAINT).
fn provider_row(state: &ConnectState, idx: usize, width: usize, theme: &ThemeColors) -> Line<'static> {
    let provider = &state.providers[idx];
    let selected = idx == state.provider_selected;
    let status = if provider.connected {
        format!("{} connected", glyphs::MARK_DONE)
    } else {
        format!("{} not connected", glyphs::DOT_HOLLOW)
    };
    let status_style = if provider.connected {
        theme.success
    } else {
        theme.text_muted
    };
    list_row(
        width,
        theme,
        selected,
        provider.display,
        &status,
        status_style,
    )
}

/// One method row: the selected row is the strong inverse bar; other rows show the method label
/// (BRIGHT) and its faint description in the meta column.
fn method_row(state: &ConnectState, idx: usize, width: usize, theme: &ThemeColors) -> Line<'static> {
    let method = state.methods[idx];
    let selected = idx == state.method_selected;
    list_row(
        width,
        theme,
        selected,
        method.label(),
        method.desc(),
        theme.text_muted,
    )
}

/// Render a selectable list row in the auth language: a strong inverse ORANGE bar with a `›` marker
/// when selected, else the `name` (BRIGHT) plus a `meta` string at [`META_COL`] in `meta_color`.
fn list_row(
    width: usize,
    theme: &ThemeColors,
    selected: bool,
    name: &str,
    meta: &str,
    meta_color: ratatui::style::Color,
) -> Line<'static> {
    if selected {
        let bg = theme.primary;
        let ink = Style::default().fg(theme.background).bg(bg);
        let edge = Style::default().fg(theme.secondary).bg(bg);
        let mut cells = vec![(' ', ink); width.max(1)];
        place_segments(&mut cells, EDGE_COL, &[(glyphs::SELECTION_EDGE.to_owned(), edge)]);
        place_segments(&mut cells, MARKER_COL, &[(glyphs::SELECT_MARKER.to_owned(), ink)]);
        place_segments(&mut cells, NAME_COL, &[(name.to_owned(), ink)]);
        place_segments(&mut cells, META_COL, &[(meta.to_owned(), ink)]);
        return cells_to_line(&cells);
    }
    let base = Style::default().fg(theme.text).bg(theme.background);
    let name_style = Style::default().fg(theme.text_bright).bg(theme.background);
    let meta_style = Style::default().fg(meta_color).bg(theme.background);
    let mut cells = vec![(' ', base); width.max(1)];
    place_segments(&mut cells, NAME_COL, &[(name.to_owned(), name_style)]);
    place_segments(&mut cells, META_COL, &[(meta.to_owned(), meta_style)]);
    cells_to_line(&cells)
}

/// The 0600 stored-credential note shown on the API-key step: green `✓` + a secret-free reminder
/// that the key is stored owner-only and never displayed.
fn stored_note_line(width: usize, theme: &ThemeColors) -> Line<'static> {
    let base = Style::default().fg(theme.text).bg(theme.background);
    let okg = Style::default().fg(theme.success).bg(theme.background);
    let dim = Style::default().fg(theme.text_dim).bg(theme.background);
    row(
        width,
        base,
        &[(
            CONTENT_COL,
            vec![
                (format!("{} ", glyphs::MARK_DONE), okg),
                ("stored to auth.json ".to_owned(), dim),
                ("(0600)".to_owned(), okg),
                (" — the key is never shown".to_owned(), dim),
            ],
        )],
    )
}

/// The key-hints footer for the active step (key tokens in dim-orange, labels in FAINT).
fn hints_line(step: Step, width: usize, theme: &ThemeColors) -> Line<'static> {
    let base = Style::default().fg(theme.text).bg(theme.background);
    let key = Style::default().fg(theme.border).bg(theme.background);
    let label = Style::default().fg(theme.text_muted).bg(theme.background);
    let hints: &[(&str, &str)] = match step {
        Step::Provider => &[
            (concat!("↑", "↓"), "select"),
            ("↵", "choose"),
            ("d", "disconnect"),
            ("esc", "close"),
        ],
        Step::Method => &[(concat!("↑", "↓"), "select"), ("↵", "choose"), ("esc", "back")],
        Step::ApiKey => &[("↵", "connect"), ("esc", "back")],
        Step::OAuthWaiting => &[("esc", "cancel")],
        Step::Connected => &[("↵", "done"), ("esc", "close")],
        Step::ConfirmDisconnect => &[("↵", "disconnect"), ("esc", "cancel")],
        Step::Disconnected => &[("↵", "done"), ("esc", "close")],
    };
    let mut segments: Vec<(String, Style)> = Vec::new();
    for (i, (k, l)) in hints.iter().enumerate() {
        if i > 0 {
            segments.push(("   ".to_owned(), label));
        }
        segments.push(((*k).to_owned(), key));
        segments.push((" ".to_owned(), label));
        segments.push(((*l).to_owned(), label));
    }
    let mut cells = vec![(' ', base); width.max(1)];
    place_segments(&mut cells, CONTENT_COL, &segments);
    cells_to_line(&cells)
}

/// A single content row: `text` in `text_style` placed at [`CONTENT_COL`] over a `base`-painted row.
fn content_row(width: usize, base: Style, text: &str, text_style: Style) -> Line<'static> {
    row(width, base, &[(CONTENT_COL, vec![(text.to_owned(), text_style)])])
}

/// One panel-interior row: a `width`-wide cell buffer painted with `base` (so every cell carries an
/// explicit background), with each styled segment group placed at its interior column.
fn row(width: usize, base: Style, groups: &[(usize, Vec<(String, Style)>)]) -> Line<'static> {
    let mut cells = vec![(' ', base); width.max(1)];
    for (col, segments) in groups {
        place_segments(&mut cells, *col, segments);
    }
    cells_to_line(&cells)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{App, Status};
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;

    fn app_with_connect() -> App {
        let mut app = App::new(Status {
            model: "google/gemini-2.5-flash".to_owned(),
            agent: "build".to_owned(),
            mode: "build".to_owned(),
            variant: "default".to_owned(),
            session: "new".to_owned(),
        });
        app.open_connect();
        app
    }

    fn press(app: &mut App, code: KeyCode) {
        app.on_key(KeyEvent::new(code, KeyModifiers::NONE));
    }

    fn render(app: &App) -> Buffer {
        let theme = ThemeColors::from_name(app.theme());
        let backend = TestBackend::new(92, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| draw_connect(frame, app, frame.area(), &theme))
            .unwrap();
        terminal.backend().buffer().clone()
    }

    fn buffer_text(buf: &Buffer) -> String {
        buf.content().iter().map(ratatui::buffer::Cell::symbol).collect()
    }

    fn unique_auth_path(tag: &str) -> std::path::PathBuf {
        let unique = format!(
            "cortex-connect-{tag}-{}-{:?}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        std::env::temp_dir().join(unique).join("auth.json")
    }

    #[test]
    fn connect_flow_lists_providers_minus_zen() {
        // VAL-CONN-002: the flow lists connectable providers (multiple real ones), and the excluded
        // Zen provider is absent everywhere.
        let buf = render(&app_with_connect());
        let text = buffer_text(&buf);
        assert!(text.contains("Connect a provider"), "panel title:\n{text}");
        assert!(text.contains("OpenRouter"), "openrouter is listed:\n{text}");
        assert!(text.contains("Anthropic") || text.contains("Azure") || text.contains("Groq"),
            "multiple providers listed:\n{text}");
        // The excluded Zen provider (either brand id) never appears.
        let zen_a = ["open", "code"].concat();
        assert!(!text.to_lowercase().contains(&zen_a), "excluded Zen id absent:\n{text}");
        assert!(!text.to_lowercase().contains("zen"), "excluded Zen id absent:\n{text}");
    }

    #[test]
    fn connect_offers_api_key_always_and_oauth_only_where_supported() {
        // VAL-CONN-003: an OAuth-capable provider offers API key + OAuth; an api-key-only provider
        // offers only API key.
        // openrouter (index 0) is api-key only.
        let mut state = ConnectState::new();
        assert_eq!(state.selected_provider().id, "openrouter");
        state.enter_provider();
        assert_eq!(state.methods, vec![Method::ApiKey], "openrouter offers only API key");

        // Move to copilot (OAuth-capable) and check both methods are offered.
        let mut state = ConnectState::new();
        let copilot = state
            .providers
            .iter()
            .position(|p| p.id == "copilot")
            .expect("copilot connectable");
        state.provider_selected = copilot;
        state.enter_provider();
        assert!(state.methods.contains(&Method::ApiKey), "API key always offered");
        assert!(state.methods.contains(&Method::OAuth), "OAuth offered for copilot");
    }

    #[test]
    fn connect_api_key_entry_is_masked_and_never_shown() {
        // VAL-CONN-004: typed key renders as `•` bullets; the raw key never appears on screen.
        let mut app = app_with_connect();
        press(&mut app, KeyCode::Enter); // provider -> method
        press(&mut app, KeyCode::Enter); // method (API key) -> key entry
        for c in "SECRETKEY12345".chars() {
            press(&mut app, KeyCode::Char(c));
        }
        let text = buffer_text(&render(&app));
        assert!(text.contains("paste API key for OpenRouter:"), "key prompt:\n{text}");
        assert!(text.contains('•'), "masked bullets present:\n{text}");
        assert!(!text.contains("SECRETKEY12345"), "raw key must never appear:\n{text}");
        assert!(!text.contains("SECRET"), "no raw key substring appears:\n{text}");
    }

    #[test]
    fn connect_stores_credential_and_marks_connected() {
        // VAL-CONN-004/005: confirming a non-empty key stores it to the 0600 auth store and the
        // provider then reads as connected (re-resolved).
        let path = unique_auth_path("store");
        let mut state = ConnectState::new();
        state.enter_provider(); // openrouter -> method
        state.enter_method(); // API key -> key entry
        for c in "sk-or-test-key".chars() {
            state.type_char(c);
        }
        assert!(state.confirm_api_key_to(&path), "credential stored");
        assert_eq!(state.step, Step::Connected, "flow advances to connected");
        assert!(
            state.providers.iter().find(|p| p.id == "openrouter").unwrap().connected,
            "openrouter reads as connected after storing"
        );
        // The stored file exists and holds the provider entry (mode 0600 is enforced by the store).
        let body = std::fs::read_to_string(&path).expect("auth file written");
        assert!(body.contains("openrouter"), "auth.json holds the provider entry");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600, "auth.json is owner-only 0600");
        }
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn connect_empty_key_shows_inline_error_and_stores_nothing() {
        // VAL-CONN-004: an empty key confirm renders the inline `✗` error and stores nothing.
        let path = unique_auth_path("empty");
        let mut state = ConnectState::new();
        state.enter_provider();
        state.enter_method();
        assert!(!state.confirm_api_key_to(&path), "empty key does not store");
        assert!(state.error.is_some(), "an inline error is set");
        assert_eq!(state.step, Step::ApiKey, "flow stays on the key step");
        assert!(!path.exists(), "no auth file written for an empty key");
    }

    #[test]
    fn connect_inline_error_uses_err_mark_never_toast() {
        // VAL-CONN (error surface): the error renders as an inline `✗ <message>` block in the panel.
        let mut app = app_with_connect();
        press(&mut app, KeyCode::Enter); // -> method
        press(&mut app, KeyCode::Enter); // -> key entry
        press(&mut app, KeyCode::Enter); // confirm empty -> inline error
        let text = buffer_text(&render(&app));
        assert!(
            text.contains(&format!("{} API key cannot be empty", glyphs::MARK_ERROR)),
            "inline ✗ error block present:\n{text}"
        );
        // No toast is raised for the connect error.
        assert!(!app.has_toasts(), "connect errors never raise a toast");
    }

    #[test]
    fn connect_panel_geometry_is_rounded_titled_and_safe() {
        // VAL-CONN-002: the connect panel uses the auth language — rounded titled panel, the shared
        // chrome wordmark, and only safe glyphs.
        let buf = render(&app_with_connect());
        let text = buffer_text(&buf);
        assert!(text.contains('╭') && text.contains('╮'), "rounded top corners:\n{text}");
        assert!(text.contains('╰') && text.contains('╯'), "rounded bottom corners");
        assert!(text.contains("Cortex Code · connect"), "shared connect chrome:\n{text}");
        assert!(!glyphs::contains_braille(&text), "no braille anywhere");
        for forbidden in ['▸', '▾', '⌘', '⬡', '“', '”', '‘', '’'] {
            assert!(!text.contains(forbidden), "forbidden glyph {forbidden:?}");
        }
        assert!(!text.contains(">_"), "no `>_` cliché");
    }

    #[test]
    fn connect_selected_provider_row_is_a_strong_inverse_bar() {
        // The highlighted provider row is a solid ORANGE fill with near-black ink, an amber `▎`
        // edge, and a `›` marker (the auth selection language).
        let theme = ThemeColors::from_name("dark");
        let state = ConnectState::new();
        let line = provider_row(&state, 0, 66, &theme);
        let edge = line
            .spans
            .iter()
            .find(|s| s.content.as_ref() == glyphs::SELECTION_EDGE)
            .expect("amber edge present");
        assert_eq!(edge.style.fg, Some(theme.secondary), "edge is AMBER");
        assert_eq!(edge.style.bg, Some(theme.primary), "edge sits on the ORANGE fill");
        let fill = line
            .spans
            .iter()
            .find(|s| s.content.contains("OpenRouter"))
            .expect("selected provider name on the fill");
        assert_eq!(fill.style.bg, Some(theme.primary), "row fill is ORANGE");
        assert_eq!(fill.style.fg, Some(theme.background), "text is near-black");
    }

    // --- Disconnect / manage connected providers (VAL-CONN-009), driven at the state level with a
    // throwaway auth store so the round-trip is deterministic and offline-safe. ---

    #[test]
    fn connect_provider_step_shows_connected_status() {
        // VAL-CONN-009: the flow shows which providers currently have a stored credential. After a
        // credential is stored, its provider row reads as connected; others read as not connected.
        let path = unique_auth_path("list-connected");
        store_credential(&path, "openrouter", "sk-or-list").expect("store openrouter");
        let mut state = ConnectState::new();
        state.refresh_connected_from(&path);
        assert!(
            state
                .providers
                .iter()
                .find(|p| p.id == "openrouter")
                .unwrap()
                .connected,
            "openrouter reads as connected"
        );
        assert!(
            state.providers.iter().any(|p| !p.connected),
            "a provider without a stored credential reads as not connected"
        );
        // The connected marker renders on the provider row (the `✓ connected` status).
        let theme = ThemeColors::from_name("dark");
        let line = provider_row(&state, 0, 66, &theme);
        let status: String = line.spans.iter().map(|s| s.content.to_string()).collect();
        assert!(
            status.contains(&format!("{} connected", glyphs::MARK_DONE)),
            "connected status is shown on the row: {status}"
        );
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn disconnect_is_confirm_before_remove_and_removes_the_credential() {
        // VAL-CONN-009: disconnecting a connected provider is confirm-before-remove — requesting a
        // disconnect moves to a confirm step WITHOUT removing anything; confirming removes the stored
        // credential (remove_stored_credential) and re-resolves so it reads as not connected.
        let path = unique_auth_path("disconnect");
        store_credential(&path, "openrouter", "sk-or-secret").expect("store openrouter");
        store_credential(&path, "groq", "gsk-other-secret").expect("store groq");
        let mut state = ConnectState::new();
        state.refresh_connected_from(&path);
        assert_eq!(state.selected_provider().id, "openrouter");
        assert!(state.selected_provider().connected, "openrouter starts connected");

        state.request_disconnect();
        assert_eq!(
            state.step,
            Step::ConfirmDisconnect,
            "requesting a disconnect confirms before removing"
        );
        assert!(path.exists(), "nothing is removed before the confirmation");

        assert!(state.disconnect_to(&path), "confirming removes the credential");
        assert_eq!(state.step, Step::Disconnected, "flow advances to the disconnected note");
        assert!(
            !state
                .providers
                .iter()
                .find(|p| p.id == "openrouter")
                .unwrap()
                .connected,
            "openrouter reads as not connected after the disconnect"
        );
        assert!(
            state
                .providers
                .iter()
                .find(|p| p.id == "groq")
                .unwrap()
                .connected,
            "the other provider stays connected"
        );
        let body = std::fs::read_to_string(&path).expect("auth file still holds the other provider");
        assert!(!body.contains("openrouter"), "openrouter entry removed from auth.json:\n{body}");
        assert!(!body.contains("sk-or-secret"), "removed secret is gone from disk:\n{body}");
        assert!(body.contains("groq"), "the other provider is preserved:\n{body}");
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn disconnecting_the_last_provider_removes_the_store_file() {
        // VAL-CONN-009: removing the only stored credential leaves no auth.json behind.
        let path = unique_auth_path("disconnect-last");
        store_credential(&path, "openrouter", "sk-or-only").expect("store openrouter");
        let mut state = ConnectState::new();
        state.refresh_connected_from(&path);
        state.request_disconnect();
        assert!(state.disconnect_to(&path), "the sole credential is removed");
        assert!(!path.exists(), "removing the last provider deletes the store file");
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn disconnect_is_a_noop_on_a_not_connected_provider() {
        // A provider without a stored credential cannot be disconnected: requesting a disconnect is a
        // no-op that stays on the provider step (there is nothing to confirm).
        let mut state = ConnectState::new();
        for provider in &mut state.providers {
            provider.connected = false;
        }
        state.request_disconnect();
        assert_eq!(
            state.step,
            Step::Provider,
            "no confirm step for a not-connected provider"
        );
    }

    #[test]
    fn cancelling_a_disconnect_keeps_the_credential() {
        // VAL-CONN-009: cancelling the confirm keeps the stored credential (confirm-before-remove).
        let path = unique_auth_path("cancel-disconnect");
        store_credential(&path, "openrouter", "sk-or-keep").expect("store openrouter");
        let mut state = ConnectState::new();
        state.refresh_connected_from(&path);
        state.request_disconnect();
        assert_eq!(state.step, Step::ConfirmDisconnect);
        state.cancel_disconnect();
        assert_eq!(state.step, Step::Provider, "cancel returns to the provider step");
        assert!(path.exists(), "cancel removes nothing");
        assert!(
            state
                .providers
                .iter()
                .find(|p| p.id == "openrouter")
                .unwrap()
                .connected,
            "openrouter stays connected after a cancelled disconnect"
        );
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn disconnect_confirm_and_done_screens_render_safely_without_secrets() {
        // VAL-CONN-009/010: the confirm + disconnected screens render in the mono-orange auth
        // language with safe glyphs and never show a secret.
        let path = unique_auth_path("disconnect-render");
        store_credential(&path, "openrouter", "sk-or-secret-xyz").expect("store openrouter");
        let mut state = ConnectState::new();
        state.refresh_connected_from(&path);
        state.request_disconnect();
        let theme = ThemeColors::from_name("dark");
        let confirm: String = panel_body_lines(&state, 64, 22, &theme)
            .iter()
            .flat_map(|line| line.spans.iter().map(|span| span.content.to_string()))
            .collect();
        assert!(
            confirm.contains("Disconnect OpenRouter"),
            "the confirm prompt names the provider:\n{confirm}"
        );
        assert!(
            !confirm.contains("sk-or-secret-xyz"),
            "no secret appears on the confirm screen:\n{confirm}"
        );
        assert!(!glyphs::contains_braille(&confirm), "no braille on the confirm screen");

        assert!(state.disconnect_to(&path));
        let done: String = panel_body_lines(&state, 64, 22, &theme)
            .iter()
            .flat_map(|line| line.spans.iter().map(|span| span.content.to_string()))
            .collect();
        assert!(
            done.contains("disconnected"),
            "the disconnected confirmation is shown:\n{done}"
        );
        assert!(
            !done.contains("sk-or-secret-xyz"),
            "no secret appears on the disconnected screen:\n{done}"
        );
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    // --- OAuth mechanics (VAL-CONN-006/007/008/010), driven against a CONTROLLED loopback
    // OAuth endpoint that exercises the REAL `mcp::oauth` engine (no third-party provider). ---

    /// A copilot provider row selected, ready to start the OAuth method.
    fn oauth_state_for_copilot() -> ConnectState {
        let mut state = ConnectState::new();
        let copilot = state
            .providers
            .iter()
            .position(|p| p.id == "copilot")
            .expect("copilot connectable");
        state.provider_selected = copilot;
        state.enter_provider();
        assert!(
            state.methods.contains(&Method::OAuth),
            "copilot offers OAuth"
        );
        state
    }

    /// Build fixture OAuth endpoints whose token endpoint points at a controlled loopback server;
    /// the callback port is left unpinned so the engine scans the reserved 3200-3299 range.
    fn fixture_endpoints(token_base: &str) -> OAuthEndpoints {
        OAuthEndpoints {
            authorize_endpoint: format!("{token_base}/authorize"),
            token_endpoint: format!("{token_base}/token"),
            client_id: "fixture-client".to_owned(),
            scopes: vec!["read:user".to_owned()],
            callback_port: None,
        }
    }

    /// A controlled loopback token server (bound on an OS-assigned ephemeral port) that answers the
    /// single token-exchange POST with `body`. Returns its base URL and the accept thread handle.
    fn spawn_token_server(body: &str) -> (String, std::thread::JoinHandle<()>) {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind token server");
        let addr = listener.local_addr().expect("token addr");
        let response_body = body.to_owned();
        let handle = std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                let mut buffer = [0u8; 4096];
                let _ = stream.read(&mut buffer);
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response_body}",
                    response_body.len()
                );
                let _ = stream.write_all(response.as_bytes());
                let _ = stream.flush();
            }
        });
        (format!("http://{addr}"), handle)
    }

    /// Poll the OAuth worker until the loopback listener is up, returning `(authorize_url,
    /// redirect_uri)`.
    fn wait_for_awaiting(state: &mut ConnectState) -> (String, String) {
        for _ in 0..250 {
            state.poll_oauth();
            if let Some(url) = state.oauth_authorize_url() {
                let redirect = state
                    .oauth
                    .as_ref()
                    .and_then(|progress| progress.redirect_uri.clone())
                    .expect("redirect uri set with the url");
                return (url.to_owned(), redirect);
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        panic!("OAuth loopback listener did not come up");
    }

    /// Poll the OAuth worker until it leaves the waiting step (connected or failed).
    fn wait_for_result(state: &mut ConnectState) {
        for _ in 0..250 {
            state.poll_oauth();
            if state.step != Step::OAuthWaiting {
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        panic!("OAuth flow did not resolve");
    }

    /// Deliver a loopback callback GET to `redirect_uri` with `code` and `state`.
    fn deliver_callback(redirect_uri: &str, code: &str, state: &str) {
        use std::io::{Read, Write};
        use std::net::TcpStream;
        let addr = redirect_uri
            .trim_start_matches("http://")
            .split('/')
            .next()
            .expect("host:port in redirect uri")
            .to_owned();
        let mut stream = TcpStream::connect(&addr).expect("connect to loopback callback");
        let request = format!(
            "GET /callback?code={code}&state={state} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n"
        );
        stream.write_all(request.as_bytes()).expect("write callback");
        let mut buffer = Vec::new();
        let _ = stream.read_to_end(&mut buffer);
    }

    /// Read a query parameter's raw value from a URL (the values used here are un-encoded hex/ids).
    fn query_param(url: &str, key: &str) -> Option<String> {
        let query = url.split_once('?')?.1;
        for pair in query.split('&') {
            if let Some((k, v)) = pair.split_once('=')
                && k == key
            {
                return Some(v.to_owned());
            }
        }
        None
    }

    /// The port of a `http://127.0.0.1:PORT/callback` redirect uri.
    fn redirect_port(redirect_uri: &str) -> u16 {
        redirect_uri
            .trim_start_matches("http://127.0.0.1:")
            .split('/')
            .next()
            .and_then(|port| port.parse::<u16>().ok())
            .expect("callback port in redirect uri")
    }

    #[test]
    fn oauth_start_builds_valid_authorization_request_and_loopback_listener() {
        // VAL-CONN-006: choosing OAuth builds a valid authorization URL (client_id + loopback
        // redirect_uri on a 3200-3299 port + CSRF state), starts the loopback callback listener, and
        // the TUI shows a "waiting for authorization" state with the URL to open.
        let (token_base, server) = spawn_token_server(r#"{"access_token":"unused"}"#);
        let path = unique_auth_path("oauth-url");
        let mut state = oauth_state_for_copilot();
        state.begin_oauth(fixture_endpoints(&token_base), path.clone(), "copilot".to_owned());
        assert_eq!(state.step, Step::OAuthWaiting, "flow enters the waiting step");

        let (authorize_url, redirect_uri) = wait_for_awaiting(&mut state);
        assert!(
            authorize_url.contains("client_id=fixture-client"),
            "authorize URL carries the client id: {authorize_url}"
        );
        assert_eq!(
            query_param(&authorize_url, "response_type").as_deref(),
            Some("code"),
            "authorization-code flow"
        );
        let csrf_state = query_param(&authorize_url, "state").expect("CSRF state present");
        assert!(!csrf_state.is_empty(), "a non-empty CSRF state is issued");
        assert!(
            redirect_uri.starts_with("http://127.0.0.1:"),
            "loopback redirect uri: {redirect_uri}"
        );
        let port = redirect_port(&redirect_uri);
        assert!(
            (3200..=3299).contains(&port),
            "callback binds in the reserved 3200-3299 range, got {port}"
        );

        // The TUI shows the "waiting for authorization" state with the URL to open.
        let theme = ThemeColors::from_name("dark");
        let body = panel_body_lines(&state, 64, 22, &theme)
            .iter()
            .flat_map(|line| line.spans.iter().map(|span| span.content.to_string()))
            .collect::<String>();
        assert!(
            body.contains("waiting for authorization"),
            "waiting state shown:\n{body}"
        );
        assert!(
            body.contains("open this URL to authorize"),
            "the URL-to-open prompt is shown:\n{body}"
        );
        // The (secret-free) URL is displayed; it wraps across rows, so match its contiguous scheme
        // prefix rather than a fragment that a wrap boundary may split.
        assert!(
            body.contains("http://127.0.0.1:"),
            "the authorization URL is shown:\n{body}"
        );

        // Clean up: unblock the (unused) token server and drop the flow.
        state.cancel_oauth();
        drop(server);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn oauth_valid_callback_exchanges_and_stores_structured_credential() {
        // VAL-CONN-007: a valid callback (matching state + code) is exchanged at the token endpoint
        // and a STRUCTURED oauth credential (type oauth, access/refresh tokens, expiry, token
        // endpoint, client id) is stored to auth.json at 0600; the flow transitions to connected.
        let (token_base, server) = spawn_token_server(
            r#"{"access_token":"fresh-oauth-token","refresh_token":"refresh-oauth-token","token_type":"Bearer","expires_in":3600}"#,
        );
        let path = unique_auth_path("oauth-store");
        let mut state = oauth_state_for_copilot();
        state.begin_oauth(fixture_endpoints(&token_base), path.clone(), "copilot".to_owned());
        let (authorize_url, redirect_uri) = wait_for_awaiting(&mut state);
        let csrf_state = query_param(&authorize_url, "state").expect("state");

        deliver_callback(&redirect_uri, "auth-code-123", &csrf_state);
        wait_for_result(&mut state);
        server.join().expect("token server joined");

        assert_eq!(state.step, Step::Connected, "flow transitions to connected");
        assert!(state.error.is_none(), "no error on success");
        assert!(
            state
                .providers
                .iter()
                .find(|p| p.id == "copilot")
                .unwrap()
                .connected,
            "copilot reads as connected after storing"
        );

        // The stored credential is a STRUCTURED oauth entry with all the fields runtime_auth reads.
        let body = std::fs::read_to_string(&path).expect("auth file written");
        for expected in [
            "oauth",
            "access_token",
            "fresh-oauth-token",
            "refresh_token",
            "expires_at",
            "token_endpoint",
            "client_id",
            "fixture-client",
        ] {
            assert!(body.contains(expected), "stored credential has `{expected}`:\n{body}");
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600, "auth.json is owner-only 0600");
        }

        // VAL-CONN-010: the token never appears on the rendered connected screen.
        let theme = ThemeColors::from_name("dark");
        let rendered = panel_body_lines(&state, 64, 22, &theme)
            .iter()
            .flat_map(|line| line.spans.iter().map(|span| span.content.to_string()))
            .collect::<String>();
        assert!(
            !rendered.contains("fresh-oauth-token") && !rendered.contains("refresh-oauth-token"),
            "no token appears on screen:\n{rendered}"
        );

        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn oauth_mismatched_state_is_rejected_and_stores_nothing() {
        // VAL-CONN-008: a callback with a mismatched CSRF state is rejected, NOTHING is stored, and a
        // clean inline error is surfaced (the flow returns to the method step, no toast).
        let (token_base, server) = spawn_token_server(r#"{"access_token":"must-not-be-stored"}"#);
        let path = unique_auth_path("oauth-badstate");
        let mut state = oauth_state_for_copilot();
        state.begin_oauth(fixture_endpoints(&token_base), path.clone(), "copilot".to_owned());
        let (_authorize_url, redirect_uri) = wait_for_awaiting(&mut state);

        // Deliver a callback with a forged (mismatched) state.
        deliver_callback(&redirect_uri, "auth-code-123", "forged-state-value");
        wait_for_result(&mut state);
        drop(server);

        assert_eq!(state.step, Step::Method, "bad state returns to the method step");
        assert!(state.error.is_some(), "an inline error is surfaced");
        assert!(
            !state
                .providers
                .iter()
                .find(|p| p.id == "copilot")
                .unwrap()
                .connected,
            "copilot is NOT connected after a rejected callback"
        );
        assert!(
            !path.exists(),
            "no credential is stored for a mismatched CSRF state"
        );
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn oauth_denied_authorization_renders_a_clean_inline_error() {
        // VAL-CONN-008: a denied authorization surfaces a clean inline `✗ error` block and stores
        // nothing (the flow returns to the method step; the inline error renders in the panel — the
        // connect surface never raises a toast, proven by `connect_inline_error_uses_err_mark_never_toast`).
        let (token_base, server) = spawn_token_server(r#"{"access_token":"unused"}"#);
        let path = unique_auth_path("oauth-denied");
        let mut state = oauth_state_for_copilot();
        state.begin_oauth(fixture_endpoints(&token_base), path.clone(), "copilot".to_owned());
        let (_authorize_url, redirect_uri) = wait_for_awaiting(&mut state);

        // A denial callback carries `error=access_denied` (no code).
        {
            use std::io::{Read, Write};
            use std::net::TcpStream;
            let addr = redirect_uri
                .trim_start_matches("http://")
                .split('/')
                .next()
                .unwrap()
                .to_owned();
            let mut stream = TcpStream::connect(&addr).expect("connect callback");
            let request = format!(
                "GET /callback?error=access_denied HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n"
            );
            stream.write_all(request.as_bytes()).expect("write callback");
            let mut buffer = Vec::new();
            let _ = stream.read_to_end(&mut buffer);
        }
        wait_for_result(&mut state);
        drop(server);

        assert_eq!(state.step, Step::Method, "a denial returns to the method step");
        assert!(state.error.is_some(), "an inline error is surfaced");
        assert!(!path.exists(), "a denied authorization stores nothing");

        // The error renders as the inline `✗ <message>` block inside the panel.
        let theme = ThemeColors::from_name("dark");
        let body = panel_body_lines(&state, 64, 22, &theme)
            .iter()
            .flat_map(|line| line.spans.iter().map(|span| span.content.to_string()))
            .collect::<String>();
        assert!(
            body.contains(glyphs::MARK_ERROR),
            "an inline ✗ error block is rendered:\n{body}"
        );
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn oauth_credential_json_is_structured_and_matches_runtime_auth_shape() {
        // The serialized credential is the structured `oauth` shape runtime_auth reads back.
        let token = mcp::StoredToken {
            access_token: "acc".to_owned(),
            refresh_token: Some("ref".to_owned()),
            token_type: "Bearer".to_owned(),
            expires_at: Some(1234),
            token_endpoint: "http://127.0.0.1:3271/token".to_owned(),
            client_id: "fixture-client".to_owned(),
        };
        let json = oauth_credential_json(&token);
        let value: serde_json::Value = serde_json::from_str(&json).expect("valid json");
        assert_eq!(value["type"], "oauth");
        assert_eq!(value["access_token"], "acc");
        assert_eq!(value["refresh_token"], "ref");
        assert_eq!(value["expires_at"], 1234);
        assert_eq!(value["token_endpoint"], "http://127.0.0.1:3271/token");
        assert_eq!(value["client_id"], "fixture-client");
    }
}
