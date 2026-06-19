mod cli;
mod tui;

use std::process::ExitCode;

use clap::Parser;

use crate::cli::{Cli, Command, RunArgs, ServeArgs};

fn main() -> ExitCode {
    let cli = Cli::parse();
    match cli.command {
        None | Some(Command::Tui(_)) => tui::launch(),
        Some(Command::Run(args)) => run_placeholder(args),
        Some(Command::Serve(args)) => serve_placeholder(args),
    }
}

fn run_placeholder(_args: RunArgs) -> ExitCode {
    eprintln!("cortex run: the headless run path is not wired up in this build yet.");
    ExitCode::FAILURE
}

fn serve_placeholder(_args: ServeArgs) -> ExitCode {
    eprintln!("cortex serve: the local server is not wired up in this build yet.");
    ExitCode::FAILURE
}
