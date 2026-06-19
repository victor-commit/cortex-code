use std::io::{self, IsTerminal};
use std::process::ExitCode;

use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::{Backend, CrosstermBackend};
use ratatui::layout::Alignment;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

/// Launch the minimal interactive TUI placeholder.
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
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, crossterm::cursor::Hide)?;

    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let loop_result = event_loop(&mut terminal);

    let restore_result = restore(&mut terminal);

    loop_result.and(restore_result)
}

fn restore<B: Backend + io::Write>(terminal: &mut Terminal<B>) -> io::Result<()> {
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        crossterm::cursor::Show
    )?;
    terminal.show_cursor()?;
    Ok(())
}

fn event_loop<B: Backend>(terminal: &mut Terminal<B>) -> io::Result<()> {
    loop {
        terminal.draw(draw)?;

        if let Event::Key(key) = event::read()? {
            if key.kind == KeyEventKind::Release {
                continue;
            }
            match key.code {
                KeyCode::Char('q') | KeyCode::Esc => break,
                KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => break,
                _ => {}
            }
        }
    }
    Ok(())
}

fn draw(frame: &mut ratatui::Frame) {
    let block = Block::default()
        .title(" Cortex Code ")
        .borders(Borders::ALL);

    let lines = vec![
        Line::from(Span::styled(
            "Cortex Code",
            Style::default().add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from("Interactive TUI (M0 placeholder)."),
        Line::from("The full experience arrives in a later milestone."),
        Line::from(""),
        Line::from(Span::styled(
            "Press q or Ctrl-C to quit.",
            Style::default().add_modifier(Modifier::DIM),
        )),
    ];

    let paragraph = Paragraph::new(lines)
        .alignment(Alignment::Center)
        .block(block);

    frame.render_widget(paragraph, frame.area());
}
