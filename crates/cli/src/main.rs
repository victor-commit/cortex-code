mod checkpoint;
mod cli;
mod config;
mod mcp_cmd;
mod net;
mod observe;
mod permission;
mod permission_store;
mod run;
mod session;
mod todo_store;
mod tooling;

use std::process::ExitCode;
use std::sync::Arc;

use clap::Parser;
use clap::error::ErrorKind;
use errors::Exit;
use events::{Event, EventBus, LifecyclePhase, Surface};

use crate::cli::{Cli, Command, ServeArgs};
use crate::observe::{CounterSink, LogSink};

fn main() -> ExitCode {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(err) => return report_parse_error(err),
    };

    let debug_events = observe::init(cli.verbose);
    let bus = Arc::new(EventBus::new());
    let counter = Arc::new(CounterSink::new());
    if debug_events {
        bus.subscribe(Arc::new(LogSink));
        bus.subscribe(counter.clone());
    }

    // A startup/lifecycle event flows through the typed bus on every invocation. With debug
    // observability enabled it reaches both attached sinks (each exactly once); with no sinks
    // attached the publish is a clean no-op.
    bus.publish(&Event::lifecycle(
        LifecyclePhase::Startup,
        surface_of(&cli.command),
        env!("CARGO_PKG_VERSION"),
    ));
    if debug_events {
        counter.report();
    }

    match cli.command {
        None | Some(Command::Tui(_)) => tui::launch(),
        Some(Command::Run(args)) => run::run(args, bus.clone()).into(),
        Some(Command::Config(args)) => config::run(args).into(),
        Some(Command::Session(args)) => session::run(args).into(),
        Some(Command::Net(args)) => net::run(args).into(),
        Some(Command::Undo(args)) => checkpoint::run(args, false).into(),
        Some(Command::Redo(args)) => checkpoint::run(args, true).into(),
        Some(Command::Mcp(args)) => mcp_cmd::run(args).into(),
        Some(Command::Serve(args)) => serve_placeholder(args).into(),
    }
}

/// Map the dispatched subcommand to the surface that emits the startup lifecycle event.
fn surface_of(command: &Option<Command>) -> Surface {
    match command {
        None | Some(Command::Tui(_)) => Surface::Tui,
        Some(Command::Run(_)) => Surface::Run,
        Some(Command::Config(_)) => Surface::Config,
        Some(Command::Session(_)) => Surface::Session,
        Some(Command::Net(_)) => Surface::Net,
        Some(Command::Undo(_)) | Some(Command::Redo(_)) => Surface::Checkpoint,
        Some(Command::Mcp(_)) => Surface::Mcp,
        Some(Command::Serve(_)) => Surface::Serve,
    }
}

/// Render a clap parse outcome to the correct stream and map it to a stable exit code.
///
/// `--help`/`--version` are not failures: clap renders them to stdout and we exit
/// [`Exit::Success`]. Every genuine argument/usage error is written to stderr only, leaving
/// stdout byte-empty for machine consumers, and exits with the stable [`Exit::Usage`] code.
fn report_parse_error(err: clap::Error) -> ExitCode {
    let _ = err.print();
    match err.kind() {
        ErrorKind::DisplayHelp
        | ErrorKind::DisplayVersion
        | ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand => Exit::Success.into(),
        _ => Exit::Usage.into(),
    }
}

fn serve_placeholder(_args: ServeArgs) -> Exit {
    eprintln!("cortex serve: the local server is not wired up in this build yet.");
    Exit::Failure
}
