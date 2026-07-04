//! The interactive provider auth/login screen (`auth_login.png`).
//!
//! Invoked by `cortex auth login` with no provider argument (on a TTY), this replaces the
//! old print-and-exit login with a full-screen TUI: the shared `✦ Cortex Code · auth · login`
//! chrome, a centered titled sign-in panel listing the real providers (openrouter, anthropic,
//! openai (codex), azure openai, github copilot) with their auth-method tag and logged-in status,
//! an arrow-navigable selection bar, a MASKED API-key input box, a 0600 stored-credential note,
//! and a key-hints footer, over the persistent 3-zone status bar.
//!
//! Security invariant: the typed key lives only in [`AuthLoginState::key_input`] and is NEVER
//! rendered or logged — the input box shows one `•` per typed character (behind a non-secret key
//! prefix hint), and storing writes straight to the 0600 credential store without echoing the key.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::Paragraph;

use providers::{AuthPathInputs, list_stored_providers, store_credential};
use tui_components::style::ThemeColors;

use crate::app::App;
use crate::glyphs;
use crate::ui::{cells_to_line, place_segments};
use crate::view;

/// The breadcrumb crumb shown after `✦ Cortex Code ·` on the auth screen (`auth · login`, both
/// segments amber).
const CRUMB: &str = "auth · login";
/// The amber title embedded in the sign-in panel's top border.
const PANEL_TITLE: &str = "Sign in — cortex auth login";
/// The faint subtitle above the provider list.
const SUBTITLE: &str = "select a provider";
/// Width (columns) of the centered sign-in panel. Sized so the full stored-credential note
/// (`✓ credential stored in ~/.local/share/cortex/auth.json (0600)`) fits on one interior row.
const PANEL_W: u16 = 68;
/// Height (rows) of the centered sign-in panel.
const PANEL_H: u16 = 18;

// Provider-row interior column layout (relative to the panel inner region).
/// The amber `▎` left edge of the selected row.
const EDGE_COL: usize = 0;
/// The `›` selection marker of the selected row.
const MARKER_COL: usize = 1;
/// The provider name column.
const NAME_COL: usize = 3;
/// The `[method]` auth-method tag column.
const METHOD_COL: usize = 20;
/// The login-status column (`✓ logged in` / `○ not logged in`).
const STATUS_COL: usize = 33;
/// Left content column for the subtitle, key label, stored note, hints, and the input box border.
const CONTENT_COL: usize = 2;

/// Which field currently has focus. Selection arrows act on the provider list; typing edits the
/// key field. `tab` cycles between them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Focus {
    /// The provider list (arrow keys move the selection).
    Providers,
    /// The API-key input box (printable keys edit the masked value).
    KeyInput,
}

/// How a provider authenticates: a masked API key or an OAuth browser flow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AuthKind {
    /// A pasted API key (masked input box).
    ApiKey,
    /// An OAuth authorization flow (no key input).
    OAuth,
}

impl AuthKind {
    /// The bracketed method tag shown in the provider row.
    const fn tag(self) -> &'static str {
        match self {
            AuthKind::ApiKey => "[api-key]",
            AuthKind::OAuth => "[oauth]",
        }
    }
}

/// A real provider offered on the auth screen. `id` is the credential-store key (so the logged-in
/// marker and storage use the authentic provider id), `env_var` is the provider's real credential
/// environment variable (a resolved credential also counts as logged-in), and `key_prefix` is a
/// non-secret format hint shown before the masked bullets.
struct ProviderSpec {
    /// The display name shown in the row (authentic provider name).
    display: &'static str,
    /// The credential-store id used for the logged-in lookup and for storing the key.
    id: &'static str,
    /// The auth method (masked key vs OAuth).
    kind: AuthKind,
    /// The provider's primary credential environment variable (a real login source).
    env_var: &'static str,
    /// A non-secret key-format prefix hint shown before the masked bullets (may be empty).
    key_prefix: &'static str,
}

/// The five real providers offered on the auth screen, in display order.
const PROVIDERS: [ProviderSpec; 5] = [
    ProviderSpec {
        display: "openrouter",
        id: "openrouter",
        kind: AuthKind::ApiKey,
        env_var: "OPENROUTER_API_KEY",
        key_prefix: "sk-or-",
    },
    ProviderSpec {
        display: "anthropic",
        id: "anthropic",
        kind: AuthKind::ApiKey,
        env_var: "ANTHROPIC_API_KEY",
        key_prefix: "sk-ant-",
    },
    ProviderSpec {
        display: "openai (codex)",
        id: "codex",
        kind: AuthKind::ApiKey,
        env_var: "OPENAI_API_KEY",
        key_prefix: "sk-",
    },
    ProviderSpec {
        display: "azure openai",
        id: "azure",
        kind: AuthKind::ApiKey,
        env_var: "AZURE_OPENAI_API_KEY",
        key_prefix: "",
    },
    ProviderSpec {
        display: "github copilot",
        id: "copilot",
        kind: AuthKind::OAuth,
        env_var: "GITHUB_COPILOT_TOKEN",
        key_prefix: "",
    },
];

/// The interactive auth-screen state: the resolved logged-in status per provider, the highlighted
/// row, the focused field, and the (never-rendered) typed key buffer.
pub struct AuthLoginState {
    /// The logged-in flag per provider (parallel to [`PROVIDERS`]).
    logged_in: Vec<bool>,
    /// The highlighted provider row.
    selected: usize,
    /// The focused field.
    focus: Focus,
    /// The raw typed API key. Held only to persist it to the 0600 store; never rendered/logged.
    key_input: String,
}

impl AuthLoginState {
    /// Build the auth-screen state, resolving each provider's real logged-in status (a stored
    /// credential or a set credential environment variable), with the first provider highlighted
    /// and the provider list focused.
    #[must_use]
    pub fn new() -> Self {
        let stored = stored_provider_ids();
        let logged_in = PROVIDERS
            .iter()
            .map(|spec| stored.iter().any(|id| id == spec.id) || env_non_empty(spec.env_var))
            .collect();
        Self {
            logged_in,
            selected: 0,
            focus: Focus::Providers,
            key_input: String::new(),
        }
    }

    /// The highlighted provider.
    fn selected_spec(&self) -> &'static ProviderSpec {
        &PROVIDERS[self.selected.min(PROVIDERS.len() - 1)]
    }

    /// Move the highlighted provider by `delta` (wrapping), but only while the provider list is
    /// focused — once the key field is focused the arrows no longer move the selection.
    pub fn move_selection(&mut self, delta: i32) {
        if self.focus != Focus::Providers {
            return;
        }
        let n = PROVIDERS.len() as i32;
        let next = (self.selected as i32 + delta).rem_euclid(n);
        self.selected = next as usize;
    }

    /// Cycle focus between the provider list and the key input. Focus only moves to the key field
    /// for an API-key provider (an OAuth provider has no key field to focus).
    pub fn toggle_focus(&mut self) {
        self.focus = match self.focus {
            Focus::Providers if self.selected_spec().kind == AuthKind::ApiKey => Focus::KeyInput,
            Focus::Providers => Focus::Providers,
            Focus::KeyInput => Focus::Providers,
        };
    }

    /// Append a typed character to the masked key (only while the key field of an API-key provider
    /// is focused).
    pub fn type_char(&mut self, c: char) {
        if self.focus != Focus::KeyInput
            || self.selected_spec().kind != AuthKind::ApiKey
            || c.is_control()
        {
            return;
        }
        self.key_input.push(c);
    }

    /// Remove the last typed character from the masked key (while the key field is focused).
    pub fn backspace(&mut self) {
        if self.focus == Focus::KeyInput {
            self.key_input.pop();
        }
    }

    /// Persist the highlighted provider's typed key to the 0600 credential store, returning whether
    /// a credential was stored. Only an API-key provider with a non-empty key stores; the key is
    /// written straight to the store and never echoed. An OAuth provider (or an empty key) is a
    /// no-op here (the OAuth browser flow is out of scope for this screen).
    #[must_use]
    pub fn store_selected(&self) -> bool {
        let spec = self.selected_spec();
        if spec.kind != AuthKind::ApiKey {
            return false;
        }
        let key = self.key_input.trim();
        if key.is_empty() {
            return false;
        }
        match AuthPathInputs::from_process().resolve() {
            Some(path) => store_credential(&path, spec.id, key).is_ok(),
            None => false,
        }
    }

    /// The number of typed key characters (drives the count of mask bullets); never the key itself.
    fn key_len(&self) -> usize {
        self.key_input.chars().count()
    }
}

impl Default for AuthLoginState {
    fn default() -> Self {
        Self::new()
    }
}

/// Render the interactive auth/login screen into `area`: the `✦ Cortex Code · auth · login` chrome
/// (breadcrumb + rule + 3-zone status bar), then the centered titled sign-in panel with the
/// provider list, masked key input, stored-credential note, and key hints.
pub fn draw_auth_login(frame: &mut Frame, app: &App, area: Rect, theme: &ThemeColors) {
    let Some(state) = app.auth_login_state() else {
        return;
    };

    let body = view::render_chrome(frame, area, app, CRUMB, theme);
    // The shared chrome only ambers the trailing crumb; the auth breadcrumb wants BOTH `auth` and
    // `login` amber (VAL-ONB-014), so overwrite row 0 with the auth-specific header.
    if area.height > 0 {
        let header_area = Rect { height: 1, ..area };
        frame.render_widget(
            Paragraph::new(auth_header_line(area.width as usize, theme))
                .style(Style::default().bg(theme.background)),
            header_area,
        );
    }
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
    let lines = panel_body_lines(state, inner.width as usize, theme);
    frame.render_widget(
        Paragraph::new(lines).style(Style::default().bg(theme.background)),
        inner,
    );
}

/// The auth breadcrumb header line: `✦` (ORANGE) + `Cortex Code` (BRIGHT) + ` · ` (GHOST) + `auth`
/// (AMBER) + ` · ` (GHOST) + `login` (AMBER). Unlike the shared view header this ambers BOTH crumb
/// segments, matching the `auth · login` chrome.
fn auth_header_line(width: usize, theme: &ThemeColors) -> Line<'static> {
    let base = Style::default().fg(theme.text).bg(theme.background);
    let orange = Style::default().fg(theme.primary).bg(theme.background);
    let bright = Style::default()
        .fg(theme.text_bright)
        .bg(theme.background)
        .add_modifier(Modifier::BOLD);
    let ghost = Style::default().fg(theme.ghost).bg(theme.background);
    let amber = Style::default().fg(theme.secondary).bg(theme.background);
    let sep = format!(" {} ", glyphs::SEPARATOR);
    let mut cells = vec![(' ', base); width.max(1)];
    place_segments(
        &mut cells,
        view::VIEW_MARGIN as usize,
        &[
            (format!("{} ", glyphs::BRAND_STAR), orange),
            ("Cortex Code".to_owned(), bright),
            (sep.clone(), ghost),
            ("auth".to_owned(), amber),
            (sep, ghost),
            ("login".to_owned(), amber),
        ],
    );
    cells_to_line(&cells)
}

/// Build the sign-in panel interior (the region inside the titled border): the subtitle, the five
/// provider rows, then the method-specific detail block (masked key input + stored note, or the
/// OAuth affordance), and the key-hints footer. `width` is the panel's inner width.
fn panel_body_lines(state: &AuthLoginState, width: usize, theme: &ThemeColors) -> Vec<Line<'static>> {
    let base = Style::default().fg(theme.text).bg(theme.background);
    let faint = Style::default().fg(theme.text_muted).bg(theme.background);

    let mut lines: Vec<Line<'static>> = Vec::with_capacity(16);
    // Row 0: `select a provider` subtitle.
    lines.push(row(width, base, &[(CONTENT_COL, vec![(SUBTITLE.to_owned(), faint)])]));
    // Row 1: blank.
    lines.push(row(width, base, &[]));
    // Rows 2..6: the five provider rows.
    for (idx, spec) in PROVIDERS.iter().enumerate() {
        lines.push(provider_row(state, idx, spec, width, theme));
    }
    // Row 7: blank.
    lines.push(row(width, base, &[]));

    let spec = state.selected_spec();
    match spec.kind {
        AuthKind::ApiKey => {
            // Row 8: `paste API key for <provider>:` prompt (reflects the selected provider).
            let label = format!("paste API key for {}:", spec.display);
            lines.push(row(
                width,
                base,
                &[(CONTENT_COL, vec![(label, Style::default().fg(theme.text).bg(theme.background))])],
            ));
            // Rows 9..11: the orange-bordered masked input box.
            let box_w = width.saturating_sub(CONTENT_COL * 2).max(4);
            lines.push(input_box_top(width, box_w, theme));
            lines.push(input_box_mid(state, spec, width, box_w, theme));
            lines.push(input_box_bottom(width, box_w, theme));
        }
        AuthKind::OAuth => {
            // Row 8: OAuth method line (NOT the api-key paste prompt).
            lines.push(row(
                width,
                base,
                &[(
                    CONTENT_COL,
                    vec![(format!("{} uses OAuth", spec.display), Style::default().fg(theme.text).bg(theme.background))],
                )],
            ));
            // Row 9: blank.
            lines.push(row(width, base, &[]));
            // Row 10: browser-authorize affordance.
            lines.push(row(
                width,
                base,
                &[(
                    CONTENT_COL,
                    vec![(
                        format!("enter authorizes {} in your browser", spec.display),
                        Style::default().fg(theme.text_dim).bg(theme.background),
                    )],
                )],
            ));
            // Row 11: blank.
            lines.push(row(width, base, &[]));
        }
    }

    // Row 12: the green `✓` 0600 stored-credential note.
    lines.push(stored_note_line(width, theme));
    // Row 13: blank.
    lines.push(row(width, base, &[]));
    // Row 14: key hints.
    lines.push(hints_line(width, theme));

    lines
}

/// Build one provider row. The selected row is a strong inverse bar (solid ORANGE fill, near-black
/// text, `›` marker, amber `▎` edge); other rows show the name (BRIGHT), the method tag (FAINT), and
/// the login status (`✓ logged in` in OKG green, or `○ not logged in` in FAINT).
fn provider_row(
    state: &AuthLoginState,
    idx: usize,
    spec: &ProviderSpec,
    width: usize,
    theme: &ThemeColors,
) -> Line<'static> {
    let logged_in = state.logged_in.get(idx).copied().unwrap_or(false);
    let selected = idx == state.selected;
    let status_text = if logged_in {
        format!("{} logged in", glyphs::MARK_DONE)
    } else {
        format!("{} not logged in", glyphs::DOT_HOLLOW)
    };

    if selected {
        // Strong inverse: the whole inner row is filled ORANGE with near-black content.
        let bg = theme.primary;
        let ink = Style::default().fg(theme.background).bg(bg);
        let edge = Style::default().fg(theme.secondary).bg(bg);
        let mut cells = vec![(' ', ink); width.max(1)];
        place_segments(&mut cells, EDGE_COL, &[(glyphs::SELECTION_EDGE.to_owned(), edge)]);
        place_segments(&mut cells, MARKER_COL, &[(glyphs::SELECT_MARKER.to_owned(), ink)]);
        place_segments(&mut cells, NAME_COL, &[(spec.display.to_owned(), ink)]);
        place_segments(&mut cells, METHOD_COL, &[(spec.kind.tag().to_owned(), ink)]);
        place_segments(&mut cells, STATUS_COL, &[(status_text, ink)]);
        return cells_to_line(&cells);
    }

    let base = Style::default().fg(theme.text).bg(theme.background);
    let name = Style::default().fg(theme.text_bright).bg(theme.background);
    let tag = Style::default().fg(theme.text_muted).bg(theme.background);
    let status_style = if logged_in {
        Style::default().fg(theme.success).bg(theme.background)
    } else {
        Style::default().fg(theme.text_muted).bg(theme.background)
    };
    let mut cells = vec![(' ', base); width.max(1)];
    place_segments(&mut cells, NAME_COL, &[(spec.display.to_owned(), name)]);
    place_segments(&mut cells, METHOD_COL, &[(spec.kind.tag().to_owned(), tag)]);
    place_segments(&mut cells, STATUS_COL, &[(status_text, status_style)]);
    cells_to_line(&cells)
}

/// The masked-input box top border (rounded, ORANGE).
fn input_box_top(width: usize, box_w: usize, theme: &ThemeColors) -> Line<'static> {
    box_border(width, box_w, theme, glyphs::BOX_TOP_LEFT, glyphs::BOX_TOP_RIGHT)
}

/// The masked-input box bottom border (rounded, ORANGE).
fn input_box_bottom(width: usize, box_w: usize, theme: &ThemeColors) -> Line<'static> {
    box_border(width, box_w, theme, glyphs::BOX_BOTTOM_LEFT, glyphs::BOX_BOTTOM_RIGHT)
}

/// A horizontal box border row (`corner` + `─`×(box_w-2) + `corner`) in the ORANGE input tone.
fn box_border(
    width: usize,
    box_w: usize,
    theme: &ThemeColors,
    left: &str,
    right: &str,
) -> Line<'static> {
    let base = Style::default().fg(theme.text).bg(theme.background);
    let orange = Style::default().fg(theme.primary).bg(theme.background);
    let mut cells = vec![(' ', base); width.max(1)];
    let inner = box_w.saturating_sub(2);
    place_segments(
        &mut cells,
        CONTENT_COL,
        &[
            (left.to_owned(), orange),
            (glyphs::BOX_HORIZONTAL.repeat(inner), orange),
            (right.to_owned(), orange),
        ],
    );
    cells_to_line(&cells)
}

/// The masked-input box content row: ORANGE side borders enclosing the non-secret key prefix hint,
/// one `•` per typed character (the key body is NEVER shown), and a `▏` caret when the field is
/// focused.
fn input_box_mid(
    state: &AuthLoginState,
    spec: &ProviderSpec,
    width: usize,
    box_w: usize,
    theme: &ThemeColors,
) -> Line<'static> {
    let base = Style::default().fg(theme.text).bg(theme.background);
    let orange = Style::default().fg(theme.primary).bg(theme.background);
    let dim = Style::default().fg(theme.text_dim).bg(theme.background);
    let text = Style::default().fg(theme.text).bg(theme.background);

    let mut cells = vec![(' ', base); width.max(1)];
    let right_border_col = CONTENT_COL + box_w.saturating_sub(1);
    place_segments(&mut cells, CONTENT_COL, &[(glyphs::BOX_VERTICAL.to_owned(), orange)]);
    place_segments(&mut cells, right_border_col, &[(glyphs::BOX_VERTICAL.to_owned(), orange)]);

    // Interior content: prefix hint + bullets + caret, clipped to keep both borders + padding.
    let content_start = CONTENT_COL + 2;
    let prefix_len = spec.key_prefix.chars().count();
    let interior = box_w.saturating_sub(4);
    let caret_focused = state.focus == Focus::KeyInput;
    let caret_reserve = usize::from(caret_focused);
    let bullet_budget = interior.saturating_sub(prefix_len + caret_reserve);
    let bullets = glyphs::BULLET.repeat(state.key_len().min(bullet_budget));

    let mut segments: Vec<(String, Style)> = Vec::new();
    if prefix_len > 0 {
        segments.push((spec.key_prefix.to_owned(), dim));
    }
    if !bullets.is_empty() {
        segments.push((bullets, text));
    }
    if caret_focused {
        segments.push((glyphs::CURSOR.to_owned(), orange));
    }
    place_segments(&mut cells, content_start, &segments);
    cells_to_line(&cells)
}

/// The stored-credential note: green `✓`, then `credential stored in <path> ` (DIM), then the
/// `(0600)` permission note in green (VAL-ONB-026).
fn stored_note_line(width: usize, theme: &ThemeColors) -> Line<'static> {
    let base = Style::default().fg(theme.text).bg(theme.background);
    let okg = Style::default().fg(theme.success).bg(theme.background);
    let dim = Style::default().fg(theme.text_dim).bg(theme.background);
    let mut cells = vec![(' ', base); width.max(1)];
    place_segments(
        &mut cells,
        CONTENT_COL,
        &[
            (format!("{} ", glyphs::MARK_DONE), okg),
            (format!("credential stored in {} ", auth_store_display_path()), dim),
            ("(0600)".to_owned(), okg),
        ],
    );
    cells_to_line(&cells)
}

/// The key-hints footer: `enter sign in   tab next field   ↑↓ select   esc cancel` with the key
/// tokens in dim-orange and the labels in FAINT (visually distinct).
fn hints_line(width: usize, theme: &ThemeColors) -> Line<'static> {
    let base = Style::default().fg(theme.text).bg(theme.background);
    let key = Style::default().fg(theme.border).bg(theme.background);
    let label = Style::default().fg(theme.text_muted).bg(theme.background);
    let hints = [
        ("enter", "sign in"),
        ("tab", "next field"),
        (concat!("↑", "↓"), "select"),
        ("esc", "cancel"),
    ];
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

/// One panel-interior row: a `width`-wide cell buffer painted with `base` (so every cell carries an
/// explicit background), with each styled segment group placed at its interior column.
fn row(width: usize, base: Style, groups: &[(usize, Vec<(String, Style)>)]) -> Line<'static> {
    let mut cells = vec![(' ', base); width.max(1)];
    for (col, segments) in groups {
        place_segments(&mut cells, *col, segments);
    }
    cells_to_line(&cells)
}

/// The credential-store path shown in the stored note, home-relativized to `~` when it lives under
/// `$HOME` (so the common default reads `~/.local/share/cortex/auth.json`, matching the design and
/// the authentic location); falls back to the canonical default when no path resolves.
fn auth_store_display_path() -> String {
    let default = "~/.local/share/cortex/auth.json";
    let Some(path) = AuthPathInputs::from_process().resolve() else {
        return default.to_owned();
    };
    if let Some(home) = std::env::var_os("HOME").map(std::path::PathBuf::from) {
        if !home.as_os_str().is_empty() {
            if let Ok(rel) = path.strip_prefix(&home) {
                return format!("~/{}", rel.display());
            }
        }
    }
    path.display().to_string()
}

/// The provider ids that currently have a stored credential in the on-disk auth store (empty when
/// no store resolves or it is empty/unreadable).
fn stored_provider_ids() -> Vec<String> {
    AuthPathInputs::from_process()
        .resolve()
        .and_then(|path| list_stored_providers(&path).ok())
        .unwrap_or_default()
}

/// Whether the environment variable `name` is set to a non-empty (trimmed) value.
fn env_non_empty(name: &str) -> bool {
    std::env::var(name)
        .ok()
        .is_some_and(|value| !value.trim().is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{App, Status};
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;

    fn app_with_auth() -> App {
        let mut app = App::new(Status {
            model: "google/gemini-2.5-flash".to_owned(),
            agent: "build".to_owned(),
            mode: "build".to_owned(),
            variant: "default".to_owned(),
            session: "new".to_owned(),
        });
        app.open_auth_login();
        app
    }

    fn press(app: &mut App, code: KeyCode) {
        app.on_key(KeyEvent::new(code, KeyModifiers::NONE));
    }

    fn buffer_text(buf: &Buffer) -> String {
        buf.content().iter().map(ratatui::buffer::Cell::symbol).collect()
    }

    fn row_text(buf: &Buffer, y: u16) -> String {
        (0..buf.area.width)
            .map(|x| buf[(x, y)].symbol())
            .collect::<Vec<_>>()
            .concat()
    }

    fn render(app: &App) -> Buffer {
        let theme = ThemeColors::from_name(app.theme());
        let backend = TestBackend::new(92, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| draw_auth_login(frame, app, frame.area(), &theme))
            .unwrap();
        terminal.backend().buffer().clone()
    }

    #[test]
    fn breadcrumb_reads_cortex_code_auth_login_both_amber() {
        // VAL-ONB-014: `✦ Cortex Code · auth · login`, with `auth` and `login` both amber.
        let theme = ThemeColors::from_name("dark");
        let line = auth_header_line(92, &theme);
        let fg_of = |needle: &str| {
            line.spans
                .iter()
                .find(|s| s.content.as_ref() == needle)
                .and_then(|s| s.style.fg)
        };
        assert_eq!(fg_of("✦ "), Some(theme.primary), "star ORANGE");
        assert_eq!(fg_of("Cortex Code"), Some(theme.text_bright), "wordmark BRIGHT");
        assert_eq!(fg_of("auth"), Some(theme.secondary), "auth crumb AMBER");
        assert_eq!(fg_of("login"), Some(theme.secondary), "login crumb AMBER");
        let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.contains("Cortex Code · auth · login"), "crumb text:\n{text}");
    }

    #[test]
    fn renders_titled_panel_and_subtitle() {
        // VAL-ONB-015/016: a rounded titled sign-in panel + `select a provider` subtitle.
        let buf = render(&app_with_auth());
        let text = buffer_text(&buf);
        assert!(text.contains('╭') && text.contains('╮'), "rounded top corners");
        assert!(text.contains('╰') && text.contains('╯'), "rounded bottom corners");
        assert!(text.contains("Sign in — cortex auth login"), "panel title:\n{text}");
        assert!(text.contains("select a provider"), "subtitle:\n{text}");
    }

    #[test]
    fn lists_all_real_providers_with_method_tags() {
        // VAL-ONB-017/018: the five real providers, each with an aligned method tag.
        let buf = render(&app_with_auth());
        let text = buffer_text(&buf);
        for name in ["openrouter", "anthropic", "openai (codex)", "azure openai", "github copilot"] {
            assert!(text.contains(name), "provider `{name}` present:\n{text}");
        }
        assert!(text.contains("[api-key]"), "api-key method tag present");
        assert!(text.contains("[oauth]"), "oauth method tag present");
    }

    #[test]
    fn status_markers_distinguish_logged_in_states() {
        // VAL-ONB-019: `✓ logged in` vs `○ not logged in` markers.
        let buf = render(&app_with_auth());
        let text = buffer_text(&buf);
        assert!(text.contains("not logged in"), "a not-logged-in row present:\n{text}");
        // The hollow marker distinguishes not-logged-in rows.
        assert!(text.contains('○'), "hollow not-logged-in marker present");
    }

    #[test]
    fn selected_row_is_a_strong_inverse_bar() {
        // VAL-ONB-020: the selected provider row is a solid ORANGE fill with near-black text, an
        // amber `▎` edge, and a `›` marker.
        let theme = ThemeColors::from_name("dark");
        let lines = panel_body_lines(&AuthLoginState::new(), 66, &theme);
        // Row index 2 is the first provider (selected by default).
        let selected = &lines[2];
        // The `▎` edge carries its own AMBER-on-ORANGE style (distinct from the ink fill).
        let edge = selected
            .spans
            .iter()
            .find(|s| s.content.as_ref() == glyphs::SELECTION_EDGE)
            .expect("amber edge present");
        assert_eq!(edge.style.fg, Some(theme.secondary), "edge is AMBER");
        assert_eq!(edge.style.bg, Some(theme.primary), "edge sits on the ORANGE fill");
        // The rest of the row is a solid ORANGE fill with near-black ink; the `›` marker and the
        // provider name share that ink run.
        let fill = selected
            .spans
            .iter()
            .find(|s| s.content.contains("openrouter"))
            .expect("selected provider name present on the fill");
        assert_eq!(fill.style.bg, Some(theme.primary), "row fill is ORANGE");
        assert_eq!(fill.style.fg, Some(theme.background), "text is near-black");
        assert!(fill.content.contains(glyphs::SELECT_MARKER), "`›` selection marker present");
    }

    #[test]
    fn key_prompt_reflects_the_selected_provider() {
        // VAL-ONB-025: the label reads `paste API key for <provider>:` for the selection.
        let buf = render(&app_with_auth());
        let text = buffer_text(&buf);
        assert!(text.contains("paste API key for openrouter:"), "key prompt:\n{text}");
    }

    #[test]
    fn typed_key_is_masked_and_never_shown() {
        // VAL-ONB-022/023: typed characters render as `•` bullets; the raw key never appears.
        let mut app = app_with_auth();
        // Focus the key field, then type a distinctive fake key.
        press(&mut app, KeyCode::Tab);
        for c in "SECRETKEY12345".chars() {
            press(&mut app, KeyCode::Char(c));
        }
        let buf = render(&app);
        let text = buffer_text(&buf);
        assert!(text.contains('•'), "masked bullets present:\n{text}");
        assert!(!text.contains("SECRETKEY12345"), "raw key must never appear:\n{text}");
        assert!(!text.contains("SECRET"), "no raw key substring appears:\n{text}");
    }

    #[test]
    fn caret_appears_only_when_key_field_is_focused() {
        // VAL-ONB-024/031: no caret while the list is focused; the caret appears after `tab`.
        let mut app = app_with_auth();
        let before = buffer_text(&render(&app));
        assert!(!before.contains('▏'), "no caret before focusing the key field");
        press(&mut app, KeyCode::Tab);
        let after = buffer_text(&render(&app));
        assert!(after.contains('▏'), "caret appears once the key field is focused");
    }

    #[test]
    fn stored_note_has_green_0600() {
        // VAL-ONB-026: a `✓ credential stored in … (0600)` note with `✓` and `(0600)` in green.
        let theme = ThemeColors::from_name("dark");
        let line = stored_note_line(120, &theme);
        let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.contains("credential stored in"), "note text:\n{text}");
        assert!(text.contains("(0600)"), "0600 permission note present");
        let perm = line
            .spans
            .iter()
            .find(|s| s.content.as_ref() == "(0600)")
            .expect("0600 span present");
        assert_eq!(perm.style.fg, Some(theme.success), "0600 note is green");
        let check = line
            .spans
            .iter()
            .find(|s| s.content.as_ref() == format!("{} ", glyphs::MARK_DONE))
            .expect("green check present");
        assert_eq!(check.style.fg, Some(theme.success), "✓ is green");
    }

    #[test]
    fn stored_note_fits_inside_the_panel() {
        // VAL-ONB-026: the whole `✓ credential stored in <path> (0600)` note (including the green
        // `(0600)`) is visible on the rendered screen — the panel is wide enough not to clip it.
        let buf = render(&app_with_auth());
        let text = buffer_text(&buf);
        assert!(text.contains("credential stored in"), "note visible:\n{text}");
        assert!(text.contains("(0600)"), "0600 note is not clipped by the panel width:\n{text}");
    }

    #[test]
    fn hints_row_lists_the_navigation_keys() {
        // VAL-ONB-027: the footer advertises enter/tab/↑↓/esc.
        let buf = render(&app_with_auth());
        let text = buffer_text(&buf);
        for hint in ["enter", "sign in", "tab", "next field", "select", "esc", "cancel"] {
            assert!(text.contains(hint), "hint `{hint}` present:\n{text}");
        }
        assert!(text.contains('↑') && text.contains('↓'), "↑↓ select hint present");
    }

    #[test]
    fn arrow_keys_move_the_selection() {
        // VAL-ONB-021: down/up move the highlighted provider row.
        let mut state = AuthLoginState::new();
        assert_eq!(state.selected, 0);
        state.move_selection(1);
        assert_eq!(state.selected, 1, "down moves to the next provider");
        state.move_selection(-1);
        assert_eq!(state.selected, 0, "up moves back");
        state.move_selection(-1);
        assert_eq!(state.selected, PROVIDERS.len() - 1, "up wraps to the last provider");
    }

    #[test]
    fn tab_moves_focus_and_arrows_then_stop_selecting() {
        // VAL-ONB-031: tab moves focus to the key field; arrows then no longer move the selection.
        let mut state = AuthLoginState::new();
        assert_eq!(state.focus, Focus::Providers);
        state.toggle_focus();
        assert_eq!(state.focus, Focus::KeyInput, "tab focuses the key field");
        state.move_selection(1);
        assert_eq!(state.selected, 0, "arrows no longer move the selection when the key field is focused");
    }

    #[test]
    fn oauth_provider_shows_no_api_key_prompt() {
        // VAL-ONB-033: selecting the OAuth provider presents OAuth, not the masked-key input.
        let mut state = AuthLoginState::new();
        state.selected = PROVIDERS.len() - 1; // github copilot (oauth)
        assert_eq!(state.selected_spec().kind, AuthKind::OAuth);
        let theme = ThemeColors::from_name("dark");
        let lines = panel_body_lines(&state, 62, &theme);
        let text: String = lines
            .iter()
            .flat_map(|l| l.spans.iter())
            .map(|s| s.content.as_ref())
            .collect();
        assert!(!text.contains("paste API key for"), "no api-key prompt for oauth:\n{text}");
        assert!(text.contains("uses OAuth"), "oauth affordance shown:\n{text}");
    }

    #[test]
    fn typing_only_edits_the_key_when_the_field_is_focused() {
        // Typing while the provider list is focused does not touch the key buffer (arrows/select).
        let mut state = AuthLoginState::new();
        state.type_char('x');
        assert_eq!(state.key_len(), 0, "no key edit while the list is focused");
        state.toggle_focus();
        state.type_char('x');
        state.type_char('y');
        assert_eq!(state.key_len(), 2, "key edits once the field is focused");
        state.backspace();
        assert_eq!(state.key_len(), 1, "backspace removes a typed char");
    }

    #[test]
    fn status_bar_persists_with_real_model() {
        // VAL-ONB-029: the 3-zone status bar remains on the bottom row with the real model id.
        let buf = render(&app_with_auth());
        let status = row_text(&buf, 29);
        assert!(status.contains("? for help"), "status bar left zone:\n{status}");
        assert!(status.contains("allow all commands"), "status bar center zone:\n{status}");
        assert!(
            status.contains("google/gemini-2.5-flash"),
            "status bar right zone shows the real model id:\n{status}"
        );
        assert!(!status.contains("opus-4.8"), "never the mockup model id");
    }

    #[test]
    fn auth_screen_uses_only_safe_glyphs() {
        // VAL-ONB-030: no braille/forbidden glyphs anywhere on the auth screen.
        let buf = render(&app_with_auth());
        let text = buffer_text(&buf);
        assert!(!glyphs::contains_braille(&text), "no braille anywhere");
        for forbidden in ['▸', '▾', '⌘', '⬡', '“', '”', '‘', '’'] {
            assert!(!text.contains(forbidden), "forbidden glyph {forbidden:?}");
        }
        assert!(!text.contains(">_"), "no `>_` cliché");
    }
}
