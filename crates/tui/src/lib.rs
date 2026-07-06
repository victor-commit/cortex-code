//! Cortex Code interactive TUI surface.
//!
//! Clean-room application (Apache-2.0) built on the ported `tui-*` framework crates
//! (the prompt editing model is `tui_widgets::TextArea`) with `ratatui`/`crossterm`
//! for terminal I/O. This scaffold renders a settled initial screen — an empty/home
//! transcript area, a bottom prompt with readline editing and soft newlines, and a
//! status footer — and exits cleanly via `/exit` or the quit keybind.

mod app;
mod attachments;
mod auth_login;
mod brand;
mod checkpoint;
mod connect;
#[cfg(test)]
mod cross_invariants;
mod diff;
mod format_bridge;
mod gallery;
mod glyphs;
mod keybind;
mod lsp_bridge;
mod mcp_runtime;
mod mention;
mod modal;
mod motd;
mod onboarding;
mod permission;
mod remote;
mod shell;
mod slash;
mod tool_card;
mod tooling;
mod turn;
mod ui;
mod view;

use std::collections::{BTreeMap, VecDeque};
use std::io::{self, IsTerminal, Write};
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use crossterm::event::{
    self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, KeyboardEnhancementFlags,
    PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::{Backend, CrosstermBackend};

pub use app::{App, Status};

/// Startup options for the interactive TUI.
#[derive(Debug, Clone, Default)]
pub struct LaunchOptions {
    /// Persisted session id to attach on startup.
    pub session: Option<String>,
    /// Loopback `cortex serve` origin used for SSE event publication, share creation, and aborts.
    pub server_url: Option<String>,
    /// Whether configured plugin resolution/runtime/hook execution is disabled for this TUI.
    pub pure_plugins: bool,
    /// Whether reduced-motion is requested (animations off; render the final static frame). Set by
    /// the `--no-anim` flag or the `CORTEX_NO_ANIM` environment preference.
    pub reduced_motion: bool,
    /// Whether to boot directly into the interactive auth/login screen (the provider picker +
    /// masked key input). Set when `cortex auth login` is invoked interactively with no provider
    /// argument on a TTY.
    pub auth_login: bool,
}

impl LaunchOptions {
    /// Build options from environment variables used by black-box TUI validation harnesses.
    #[must_use]
    pub fn from_env() -> Self {
        Self {
            session: env_non_empty("CORTEX_SESSION"),
            server_url: env_non_empty("CORTEX_SERVER_URL"),
            pure_plugins: env_pure_enabled(),
            reduced_motion: env_no_anim_enabled(),
            auth_login: false,
        }
    }
}

/// Launches the interactive TUI, returning a process exit code.
///
/// Requires both stdin and stdout to be a TTY; otherwise it fails fast with a clear
/// message instead of blocking on a terminal that cannot deliver key events.
pub fn launch() -> ExitCode {
    launch_with_options(LaunchOptions::from_env())
}

/// Launch the interactive TUI with explicit startup options.
pub fn launch_with_options(options: LaunchOptions) -> ExitCode {
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        eprintln!(
            "cortex: the interactive TUI requires a TTY on stdin and stdout. \
             Use `cortex run` for non-interactive use, or `cortex --help` for all commands."
        );
        return ExitCode::FAILURE;
    }

    match run(options) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("cortex: TUI error: {err}");
            ExitCode::FAILURE
        }
    }
}

fn run(options: LaunchOptions) -> io::Result<()> {
    let mut app = App::new(initial_status());
    app.set_reduced_motion(options.reduced_motion);
    app::apply_startup_attention_and_theme_diagnostics(&mut app);
    // Honor a persisted theme choice so colors match the last selection across restarts.
    app.set_theme(app::active_theme_name());
    // Apply any user keybind overrides from config so custom/leader bindings take effect at launch.
    let (keybinds, keybind_diagnostics) = keybind::Keybinds::load_with_diagnostics();
    app.set_keybinds(keybinds);
    app.set_keybind_diagnostics(keybind_diagnostics);
    // Open the shared shadow checkpoint store for the working directory so `/undo` and `/redo` can
    // revert/re-apply file-affecting turns on disk. It degrades to graceful no-ops when no data
    // location resolves; opening never blocks launch.
    let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    app.set_checkpoint_store(checkpoints::ShadowStore::open_default(&cwd).ok().flatten());
    // Bind durable session persistence to the shared store so interactive turns are recorded and
    // become resumable from headless `cortex run` and the session picker (cross-surface continuity).
    app.set_session_store(db::DbPathInputs::from_process().resolve());
    app.set_plugin_text_complete(load_plugin_text_complete_hooks(options.pure_plugins));
    let server_url = options
        .server_url
        .or_else(|| env_non_empty("CORTEX_SERVER_URL"));
    if let Some(url) = server_url {
        match remote::RemoteServer::from_url(&url) {
            Ok(server) => app.set_remote_server(Some(server)),
            Err(err) => app.note_error(format!("Could not attach to server: {err}")),
        }
    }
    let initial_session = options.session.or_else(|| env_non_empty("CORTEX_SESSION"));
    let has_initial_session = initial_session.is_some();
    if let Some(session) = initial_session {
        app.attach_session(&session);
    }
    // On a pristine first run (a fresh HOME/XDG with no user config and no stored credentials) and
    // no explicit session to resume, boot into the onboarding welcome screen; a configured
    // environment (or a resumed session) goes straight to the normal home/MOTD surface.
    if !has_initial_session {
        app.set_onboarding(onboarding::is_first_run());
    }
    // When launched as the interactive `cortex auth login` surface, boot straight into the auth/
    // login screen (provider picker + masked key input); it takes over the frame until the user
    // signs in or cancels. This supersedes any onboarding splash.
    if options.auth_login {
        app.set_onboarding(false);
        app.open_auth_login();
    }
    // Connect configured MCP servers once for the attached TUI so `/mcp` and tool materialization
    // share a live catalogue. Runtime `mcp.tools.changed` notifications flow into the app state and
    // update the visible MCP view without a restart.
    if let Some(runtime) = mcp_runtime::connect_from_config() {
        app.set_mcp_runtime(
            Some(runtime.manager),
            Some(runtime.runtime),
            Some(runtime.events),
        );
    }

    // Kick off a non-blocking models.dev refresh at STARTUP so the `/models` picker reflects the
    // current catalogue on its first open (the background worker warms the on-disk cache and this
    // session's catalogue). Fail-closed under `CORTEX_NO_NET` (spawns nothing); the poll hook in the
    // event loop drains the result.
    app.spawn_models_refresh();

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    // Opt into the kitty keyboard protocol so modified keys (notably Shift+Enter)
    // are disambiguated from their legacy encodings. Harmless if unsupported.
    let _ = execute!(
        stdout,
        PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
    );

    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let loop_result = event_loop(&mut terminal, &mut app, options.pure_plugins);

    if let Some((manager, runtime)) = app.take_mcp_runtime() {
        mcp_runtime::shutdown(manager, runtime);
    }

    let restore_result = restore(&mut terminal);

    loop_result.and(restore_result)
}

fn restore<B: Backend + Write>(terminal: &mut Terminal<B>) -> io::Result<()> {
    let _ = execute!(terminal.backend_mut(), PopKeyboardEnhancementFlags);
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    Ok(())
}

/// Copy `text` to the terminal's clipboard using the OSC 52 escape sequence. This is a real,
/// dependency-free clipboard mechanism honored by modern terminals; the sequence is non-printing,
/// so it does not disturb the rendered screen.
fn copy_to_clipboard<B: Backend + Write>(terminal: &mut Terminal<B>, text: &str) -> io::Result<()> {
    let sequence = format!("\x1b]52;c;{}\x07", base64_encode(text.as_bytes()));
    let backend = terminal.backend_mut();
    backend.write_all(sequence.as_bytes())?;
    Write::flush(backend)
}

/// Standard-alphabet Base64 encoding (with `=` padding), used for the OSC 52 clipboard payload. Kept
/// inline to avoid pulling in a dependency for a single small encoder.
fn base64_encode(input: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let b0 = u32::from(chunk[0]);
        let b1 = u32::from(*chunk.get(1).unwrap_or(&0));
        let b2 = u32::from(*chunk.get(2).unwrap_or(&0));
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(TABLE[((n >> 18) & 63) as usize] as char);
        out.push(TABLE[((n >> 12) & 63) as usize] as char);
        out.push(if chunk.len() > 1 {
            TABLE[((n >> 6) & 63) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            TABLE[(n & 63) as usize] as char
        } else {
            '='
        });
    }
    out
}

/// Short probe window used to detect Esc-prefixed arrow sequences emitted by terminals that do not
/// report modified arrows as a single key event.
const ESC_SEQUENCE_POLL_WINDOW: Duration = Duration::from_millis(20);
/// Upper bound for the CSI/SS3 suffix length we sample after `Esc` (`[A`, `[1;3B`, `OA`, ...).
const ESC_SEQUENCE_MAX_SUFFIX_LEN: usize = 8;

/// Detect whether an Esc-prefixed CSI/SS3 suffix encodes a vertical arrow (`Up`/`Down`).
fn is_esc_prefixed_vertical_arrow(suffix: &str) -> bool {
    let bytes = suffix.as_bytes();
    if bytes.len() < 2 {
        return false;
    }
    let start = bytes[0];
    if start != b'[' && start != b'O' {
        return false;
    }
    let Some(&end) = bytes.last() else {
        return false;
    };
    if end != b'A' && end != b'B' {
        return false;
    }
    bytes[1..bytes.len() - 1]
        .iter()
        .all(|b| b.is_ascii_digit() || *b == b';')
}

/// Parse one Esc-prefixed key event from crossterm and normalize known noisy alt-arrow encodings.
///
/// Returns:
/// - `Some(key)` for a key that should be delivered to `App::on_key`.
/// - `None` when an Esc-prefixed vertical-arrow sequence was consumed as inert noise.
///
/// Any lookahead keys that are not part of a consumed sequence are queued in `pending_keys` so
/// they are still delivered in order.
fn normalize_esc_prefixed_key(
    key: KeyEvent,
    pending_keys: &mut VecDeque<KeyEvent>,
) -> io::Result<Option<KeyEvent>> {
    if key.code != KeyCode::Esc || key.modifiers != KeyModifiers::NONE {
        return Ok(Some(key));
    }
    if !event::poll(ESC_SEQUENCE_POLL_WINDOW)? {
        return Ok(Some(key));
    }
    let Event::Key(next) = event::read()? else {
        return Ok(Some(key));
    };
    if next.kind == KeyEventKind::Release {
        pending_keys.push_back(next);
        return Ok(Some(key));
    }
    if next.modifiers != KeyModifiers::NONE {
        pending_keys.push_back(next);
        return Ok(Some(key));
    }
    match next.code {
        KeyCode::Up | KeyCode::Down => Ok(None),
        KeyCode::Char('A') | KeyCode::Char('B') => {
            if !event::poll(ESC_SEQUENCE_POLL_WINDOW)? {
                pending_keys.push_back(next);
                return Ok(Some(key));
            }
            let Event::Key(peek) = event::read()? else {
                pending_keys.push_back(next);
                return Ok(Some(key));
            };
            if peek.kind == KeyEventKind::Release {
                pending_keys.push_back(next);
                pending_keys.push_back(peek);
                return Ok(Some(key));
            }
            if peek.modifiers == KeyModifiers::NONE
                && matches!(peek.code, KeyCode::Char('[') | KeyCode::Char('O'))
            {
                return Ok(None);
            }
            pending_keys.push_back(next);
            pending_keys.push_back(peek);
            Ok(Some(key))
        }
        KeyCode::Char('[') | KeyCode::Char('O') => {
            let mut consumed = vec![next];
            let mut suffix = match next.code {
                KeyCode::Char(c) => c.to_string(),
                _ => String::new(),
            };
            while suffix.len() < ESC_SEQUENCE_MAX_SUFFIX_LEN {
                if !event::poll(ESC_SEQUENCE_POLL_WINDOW)? {
                    break;
                }
                let Event::Key(peek) = event::read()? else {
                    break;
                };
                if peek.kind == KeyEventKind::Release {
                    continue;
                }
                if peek.modifiers != KeyModifiers::NONE {
                    pending_keys.push_back(peek);
                    break;
                }
                let KeyCode::Char(ch) = peek.code else {
                    pending_keys.push_back(peek);
                    break;
                };
                suffix.push(ch);
                consumed.push(peek);
                if ch.is_ascii_alphabetic() || ch == '~' {
                    break;
                }
            }
            if is_esc_prefixed_vertical_arrow(&suffix) {
                return Ok(None);
            }
            for key in consumed {
                pending_keys.push_back(key);
            }
            Ok(Some(key))
        }
        _ => {
            pending_keys.push_back(next);
            Ok(Some(key))
        }
    }
}

fn event_loop<B: Backend + Write>(
    terminal: &mut Terminal<B>,
    app: &mut App,
    pure_plugins: bool,
) -> io::Result<()> {
    let mut pending_keys = VecDeque::new();
    loop {
        terminal.draw(|frame| ui::draw(frame, app))?;
        if app.should_quit {
            break;
        }

        // Start a streaming turn for any prompt the last Enter submitted. The worker runs on its
        // own thread and reports events back over a channel; redraw immediately so the user message
        // and busy indicator appear before the first delta arrives. The shared cancel flag is the
        // interrupt seam: the UI sets it (Esc) to stop the turn gracefully.
        if let Some(prompt) = app.take_pending_prompt() {
            let user_parts = app.pending_user_parts_for_turn();
            let cancel = Arc::new(AtomicBool::new(false));
            let (session_id, db_path) = app.tool_store_context_for_turn();
            let remote = app.remote_bridge_for_turn();
            let rx = turn::spawn(turn::SpawnRequest {
                prompt: prompt.clone(),
                user_parts,
                history: app.history_snapshot(),
                model: app.active_model(),
                agent: app.active_agent(),
                variant: app.active_variant(),
                pure_plugins,
                mcp_manager: app.mcp_manager(),
                remote,
                session_id,
                db_path,
                cancel: cancel.clone(),
            });
            app.begin_turn(prompt, rx, cancel);
            continue;
        }

        // Poll for input with the animation-tick cadence while a turn streams (so the spinner
        // animates and deltas are applied promptly) or while a toast is on screen (so it
        // auto-dismisses on time and its depletion animation stays smooth), and the cheap idle
        // cadence otherwise so the loop blocks rather than busy-spinning. Reduced-motion renders a
        // static frame, but a turn/toast still polls at the active cadence to apply deltas.
        let timeout = app.poll_timeout();
        if let Some(key) = pending_keys.pop_front() {
            app.on_key(key);
        } else if event::poll(timeout)? {
            match event::read()? {
                Event::Key(key) => {
                    if key.kind == KeyEventKind::Release {
                        continue;
                    }
                    if let Some(normalized) = normalize_esc_prefixed_key(key, &mut pending_keys)? {
                        app.on_key(normalized);
                    }
                }
                // Resize is handled implicitly: the next draw recomputes the layout.
                Event::Resize(_, _) => {}
                _ => {}
            }
        }

        // Honor a `/editor` request: suspend the TUI, open the external editor, and land the
        // composed text back in the prompt before the next draw.
        if app.take_editor_request() {
            compose_in_editor(terminal, app)?;
        }

        // Honor a pending message-copy by writing the text to the terminal clipboard via OSC 52.
        // The on-screen acknowledgement is shown by the app; this performs the real copy.
        if let Some(text) = app.take_copy_request() {
            let _ = copy_to_clipboard(terminal, &text);
        }

        // Apply a server-side abort request before draining new deltas so the stream halts promptly.
        app.poll_remote_abort();

        // Apply any streamed turn events (text/reasoning growth, usage, completion) to the state.
        app.poll_turn();

        // Advance the mid-session `/connect` OAuth handshake: drain the background worker so the
        // "waiting for authorization" screen transitions to connected (or an inline error) once the
        // loopback callback is exchanged, without a restart.
        app.poll_connect_oauth();

        // Drain the background models.dev refresh worker so a live catalogue fetched off-thread
        // swaps into the `/models` picker in place (and clears the "refreshing…" indicator) without
        // ever blocking this synchronous loop. Fail-closed/again-offline results degrade silently.
        app.poll_models_refresh();

        // Tail every backgrounded `bash` card's log into its body and complete it once the job's
        // pid has exited, so a running background card stays live-updating instead of the static
        // "job started" acknowledgement (VAL-TOOL-023).
        app.poll_background_jobs();

        // Apply MCP live catalogue events (notably `mcp.tools.changed`) so an open `/mcp` view is
        // refreshed from the manager's current tool set without relaunching the TUI.
        app.poll_mcp();

        // Pull in messages appended to the attached session by the server surface.
        app.poll_remote_session();

        // Apply any TUI control requests an attached server has queued (append/submit/clear prompt,
        // open the model picker, show a toast, run a command, switch sessions).
        app.poll_remote_control();

        // Expire/resolve any Esc-prefix probe so buffered text or inert residues do not linger.
        app.tick_nav_escape_probe();

        // Expire any toasts whose display window has elapsed so they auto-dismiss on a timer.
        app.tick_toasts();
        app.tick_attention();

        let bells = app.drain_pending_bells();
        if bells > 0 {
            let backend = terminal.backend_mut();
            for _ in 0..bells {
                backend.write_all(b"\x07")?;
            }
            Write::flush(backend)?;
        }

        if app.should_quit {
            break;
        }
    }
    Ok(())
}

fn env_non_empty(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .filter(|value| !value.trim().is_empty())
}

fn env_pure_enabled() -> bool {
    std::env::var_os("CORTEX_PURE").is_some_and(|value| value.to_string_lossy() == "1")
}

/// Whether the `CORTEX_NO_ANIM` reduced-motion preference is set. Accepts `1`/`true` (any case) as
/// truthy so the no-subcommand TUI launch can honor reduced-motion without the `--no-anim` flag.
///
/// This is the single source of truth for parsing the environment reduced-motion preference; the
/// `--no-anim` flag OR-combines with it in the CLI dispatch (`crates/cli/src/main.rs`).
pub fn env_no_anim_enabled() -> bool {
    std::env::var_os("CORTEX_NO_ANIM").is_some_and(|value| {
        let value = value.to_string_lossy().to_ascii_lowercase();
        matches!(value.as_str(), "1" | "true")
    })
}

fn load_plugin_text_complete_hooks(pure_plugins: bool) -> Option<plugins::PluginEventHooks> {
    let inputs = settings::ConfigInputs::from_process(BTreeMap::new()).ok()?;
    let plugin_cwd = inputs.cwd.clone();
    let effective = settings::load(inputs).ok()?;
    let plugin_config =
        plugins::PluginConfig::from_toml_value(effective.get("plugin"), &plugin_cwd).ok()?;
    plugins::PluginEventHooks::load_or_disabled(&plugin_config, pure_plugins, "tui").ok()
}

/// Suspend the TUI, open the user's external editor to compose prompt text, then restore the TUI
/// and land the composed text back in the prompt.
///
/// The terminal is handed back to the editor (raw mode + alternate screen left), the editor runs to
/// completion, and the TUI's terminal modes are re-entered with a forced full redraw. A clean editor
/// exit replaces the prompt with the composed text; an aborted edit (non-zero exit) leaves it
/// unchanged, and an I/O failure surfaces a transcript error rather than crashing the loop.
fn compose_in_editor<B: Backend + Write>(
    terminal: &mut Terminal<B>,
    app: &mut App,
) -> io::Result<()> {
    let seed = app.input_value();

    let _ = execute!(terminal.backend_mut(), PopKeyboardEnhancementFlags);
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;

    let result = app::run_editor(&seed);

    enable_raw_mode()?;
    execute!(terminal.backend_mut(), EnterAlternateScreen)?;
    let _ = execute!(
        terminal.backend_mut(),
        PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
    );
    terminal.clear()?;

    match result {
        Ok(Some(text)) => app.set_input_value(&text),
        Ok(None) => {}
        Err(err) => app.note_error(format!("editor failed: {err}")),
    }
    Ok(())
}

/// Builds the initial footer status from effective configuration, falling back to
/// documented defaults so the TUI always launches even if config loading fails.
fn initial_status() -> Status {
    let effective = settings::ConfigInputs::from_process(BTreeMap::new())
        .ok()
        .and_then(|inputs| settings::load(inputs).ok());

    let model = effective
        .as_ref()
        .map(|cfg| cfg.model().to_owned())
        .unwrap_or_else(|| settings::DEFAULT_MODEL.to_owned());

    let agent = effective
        .as_ref()
        .map(|cfg| cfg.agent().to_owned())
        .unwrap_or_else(|| settings::DEFAULT_AGENT.to_owned());

    let mode = effective
        .as_ref()
        .and_then(|cfg| cfg.get("mode"))
        .and_then(|value| value.as_str())
        .unwrap_or("build")
        .to_owned();

    let variant = effective
        .as_ref()
        .and_then(|cfg| cfg.get("variant"))
        .and_then(|value| value.as_str())
        .unwrap_or("default")
        .to_owned();

    Status {
        model,
        agent,
        mode,
        variant,
        session: "new".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::is_esc_prefixed_vertical_arrow;

    #[test]
    fn detects_vertical_arrow_suffixes() {
        assert!(is_esc_prefixed_vertical_arrow("[A"));
        assert!(is_esc_prefixed_vertical_arrow("[B"));
        assert!(is_esc_prefixed_vertical_arrow("OA"));
        assert!(is_esc_prefixed_vertical_arrow("OB"));
        assert!(is_esc_prefixed_vertical_arrow("[1;3A"));
        assert!(is_esc_prefixed_vertical_arrow("[1;9B"));
    }

    #[test]
    fn ignores_non_vertical_or_malformed_suffixes() {
        assert!(!is_esc_prefixed_vertical_arrow(""));
        assert!(!is_esc_prefixed_vertical_arrow("["));
        assert!(!is_esc_prefixed_vertical_arrow("[C"));
        assert!(!is_esc_prefixed_vertical_arrow("[1;3C"));
        assert!(!is_esc_prefixed_vertical_arrow("[xA"));
        assert!(!is_esc_prefixed_vertical_arrow("A"));
    }
}
