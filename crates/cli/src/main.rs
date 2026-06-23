mod acp_cmd;
mod agent_cmd;
mod auth_cmd;
mod checkpoint;
mod cli;
mod config;
mod custom_agents;
mod export_import;
mod format_bridge;
mod generate_cmd;
mod lsp_bridge;
mod lsp_cmd;
mod mcp_cmd;
mod mcp_store;
mod models_cmd;
mod net;
mod observe;
mod permission;
mod permission_store;
mod plugin_bridge;
mod replay;
mod run;
mod search;
mod serve_cmd;
mod session;
mod stats;
mod subagent;
mod todo_store;
mod tooling;

use std::process::ExitCode;
use std::sync::Arc;

use clap::Parser;
use clap::error::ErrorKind;
use errors::Exit;
use events::{Event, EventBus, LifecyclePhase, Surface};

use crate::cli::{Cli, Command};
use crate::observe::{CounterSink, LogSink};

fn main() -> ExitCode {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(err) => return report_parse_error(err),
    };

    let debug_events = observe::init(cli.verbose);
    let pure_plugins = cli.pure || plugin_bridge::env_pure_enabled();
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
        None => {
            let mut options = tui::LaunchOptions::from_env();
            options.pure_plugins = pure_plugins;
            tui::launch_with_options(options)
        }
        Some(Command::Tui(args)) => tui::launch_with_options(tui::LaunchOptions {
            session: args.session,
            server_url: args.server,
            pure_plugins,
        }),
        Some(Command::Run(args)) => run::run(args, bus.clone(), pure_plugins).into(),
        Some(Command::Acp(args)) => acp_cmd::run(args, bus.clone()).into(),
        Some(Command::Config(args)) => config::run(args).into(),
        Some(Command::Session(args)) => session::run(args).into(),
        Some(Command::Export(args)) => export_import::run_export(args).into(),
        Some(Command::Import(args)) => export_import::run_import(args).into(),
        Some(Command::Stats(args)) => stats::run(args).into(),
        Some(Command::Search(args)) => search::run(args).into(),
        Some(Command::Replay(args)) => replay::run(args).into(),
        Some(Command::Agent(args)) => agent_cmd::run(args).into(),
        Some(Command::Auth(args)) => auth_cmd::run(args, pure_plugins).into(),
        Some(Command::Models(args)) => models_cmd::run(args, pure_plugins).into(),
        Some(Command::Generate(args)) => generate_cmd::run(args).into(),
        Some(Command::Net(args)) => net::run(args).into(),
        Some(Command::Undo(args)) => checkpoint::run(args, false).into(),
        Some(Command::Redo(args)) => checkpoint::run(args, true).into(),
        Some(Command::Mcp(args)) => mcp_cmd::run(args).into(),
        Some(Command::Lsp(args)) => lsp_cmd::run(args).into(),
        Some(Command::Serve(args)) => serve_cmd::run(args, bus.clone(), pure_plugins).into(),
    }
}

/// Map the dispatched subcommand to the surface that emits the startup lifecycle event.
fn surface_of(command: &Option<Command>) -> Surface {
    match command {
        None | Some(Command::Tui(_)) => Surface::Tui,
        Some(Command::Run(_)) => Surface::Run,
        Some(Command::Acp(_)) => Surface::Acp,
        Some(Command::Config(_)) => Surface::Config,
        Some(Command::Session(_)) => Surface::Session,
        Some(Command::Export(_)) | Some(Command::Import(_)) => Surface::Session,
        Some(Command::Stats(_)) => Surface::Session,
        Some(Command::Search(_)) => Surface::Session,
        Some(Command::Replay(_)) => Surface::Session,
        Some(Command::Agent(_)) | Some(Command::Generate(_)) => Surface::Agent,
        Some(Command::Auth(_)) => Surface::Auth,
        Some(Command::Models(_)) => Surface::Models,
        Some(Command::Net(_)) => Surface::Net,
        Some(Command::Undo(_)) | Some(Command::Redo(_)) => Surface::Checkpoint,
        Some(Command::Mcp(_)) => Surface::Mcp,
        Some(Command::Lsp(_)) => Surface::Lsp,
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
