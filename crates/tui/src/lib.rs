//! Cortex Code interactive TUI surface.
//!
//! Clean-room application (Apache-2.0) built on the ported `tui-*` framework crates
//! (the prompt editing model is `tui_widgets::TextArea`) with `ratatui`/`crossterm`
//! for terminal I/O. This scaffold renders a settled initial screen — an empty/home
//! transcript area, a bottom prompt with readline editing and soft newlines, and a
//! status footer — and exits cleanly via `/exit` or the quit keybind.

mod app;
mod diff;
mod mention;
mod shell;
mod slash;
mod tool_card;
mod tooling;
mod turn;
mod ui;

use std::collections::BTreeMap;
use std::io::{self, IsTerminal, Write};
use std::process::ExitCode;
use std::time::Duration;

use crossterm::event::{
    self, Event, KeyEventKind, KeyboardEnhancementFlags, PopKeyboardEnhancementFlags,
    PushKeyboardEnhancementFlags,
};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::{Backend, CrosstermBackend};

pub use app::{App, Status};

/// Launches the interactive TUI, returning a process exit code.
///
/// Requires both stdin and stdout to be a TTY; otherwise it fails fast with a clear
/// message instead of blocking on a terminal that cannot deliver key events.
pub fn launch() -> ExitCode {
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        eprintln!(
            "cortex: the interactive TUI requires a TTY on stdin and stdout. \
             Use `cortex run` for non-interactive use, or `cortex --help` for all commands."
        );
        return ExitCode::FAILURE;
    }

    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("cortex: TUI error: {err}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> io::Result<()> {
    let mut app = App::new(initial_status());
    // Honor a persisted theme choice so colors match the last selection across restarts.
    app.set_theme(app::active_theme_name());

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

    let loop_result = event_loop(&mut terminal, &mut app);

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

fn event_loop<B: Backend + Write>(terminal: &mut Terminal<B>, app: &mut App) -> io::Result<()> {
    loop {
        terminal.draw(|frame| ui::draw(frame, app))?;
        if app.should_quit {
            break;
        }

        // Start a streaming turn for any prompt the last Enter submitted. The worker runs on its
        // own thread and reports events back over a channel; redraw immediately so the user message
        // and busy indicator appear before the first delta arrives.
        if let Some(prompt) = app.take_pending_prompt() {
            let rx = turn::spawn(prompt.clone(), app.history_snapshot());
            app.begin_turn(prompt, rx);
            continue;
        }

        // Poll for input with a short timeout while a turn streams (so the spinner animates and
        // deltas are applied promptly) and a longer one when idle (so the loop stays cheap).
        let timeout = if app.is_busy() {
            Duration::from_millis(50)
        } else {
            Duration::from_millis(200)
        };
        if event::poll(timeout)? {
            match event::read()? {
                Event::Key(key) => {
                    if key.kind == KeyEventKind::Release {
                        continue;
                    }
                    app.on_key(key);
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

        // Apply any streamed turn events (text/reasoning growth, usage, completion) to the state.
        app.poll_turn();

        if app.should_quit {
            break;
        }
    }
    Ok(())
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
        .and_then(|cfg| cfg.get("agent"))
        .and_then(|value| value.as_str())
        .unwrap_or("build")
        .to_owned();

    let mode = effective
        .as_ref()
        .and_then(|cfg| cfg.get("mode"))
        .and_then(|value| value.as_str())
        .unwrap_or("build")
        .to_owned();

    Status {
        model,
        agent,
        mode,
        session: "new".to_owned(),
    }
}
