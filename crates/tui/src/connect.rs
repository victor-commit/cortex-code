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
//!    it (the OAuth handshake itself is wired by the connect-oauth-bridge feature; here the method
//!    is presented).
//! 3. **API key** — a MASKED entry (never echoed); confirming stores it via
//!    [`providers::store_credential`] to `auth.json` (0600) and RE-RESOLVES so the provider reads as
//!    connected immediately (usable in `/models` with no restart).
//!
//! Security invariant: the typed key lives only in [`ConnectState::key_input`] and is NEVER rendered
//! or logged — the input box shows one `•` per typed character, and storing writes straight to the
//! 0600 credential store without echoing the key. Failures render as the inline `✗ error` block
//! inside the panel (the VAL-ERR error surface), never a toast/splatter.

use std::path::Path;

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::widgets::Paragraph;

use providers::{
    AuthPathInputs, connectable_providers, connected_providers, list_stored_providers,
    store_credential,
};
use tui_components::style::ThemeColors;

use crate::app::App;
use crate::auth_login::masked_input_box;
use crate::glyphs;
use crate::ui::{cells_to_line, place_segments};
use crate::view;

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
    /// The provider has been connected (success confirmation).
    Connected,
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
    /// A faint inline notice (e.g. the OAuth method is presented but its handshake is wired by the
    /// connect-oauth-bridge feature).
    notice: Option<String>,
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

    /// Act on the highlighted method: API key opens the masked entry; OAuth is presented (its
    /// handshake is wired by the connect-oauth-bridge feature) with a faint inline notice.
    pub fn enter_method(&mut self) {
        match self.methods.get(self.method_selected).copied() {
            Some(Method::ApiKey) => {
                self.key_input.clear();
                self.error = None;
                self.notice = None;
                self.step = Step::ApiKey;
            }
            Some(Method::OAuth) => {
                self.notice = Some(format!(
                    "OAuth sign-in for {} authorizes in your browser.",
                    self.selected_provider().display
                ));
            }
            None => {}
        }
    }

    /// Return to the provider step from the method step.
    pub fn back_to_provider(&mut self) {
        self.error = None;
        self.notice = None;
        self.step = Step::Provider;
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
        Step::Provider => &[(concat!("↑", "↓"), "select"), ("↵", "choose"), ("esc", "close")],
        Step::Method => &[(concat!("↑", "↓"), "select"), ("↵", "choose"), ("esc", "back")],
        Step::ApiKey => &[("↵", "connect"), ("esc", "back")],
        Step::Connected => &[("↵", "done"), ("esc", "close")],
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
}
