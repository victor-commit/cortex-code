mod cli;
mod config;
mod run;
mod tui;

use std::process::ExitCode;

use clap::Parser;
use clap::error::ErrorKind;
use cortex_errors::Exit;

use crate::cli::{Cli, Command, ServeArgs};

fn main() -> ExitCode {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(err) => return report_parse_error(err),
    };

    match cli.command {
        None | Some(Command::Tui(_)) => tui::launch(),
        Some(Command::Run(args)) => run::run(args).into(),
        Some(Command::Config(args)) => config::run(args).into(),
        Some(Command::Serve(args)) => serve_placeholder(args).into(),
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
