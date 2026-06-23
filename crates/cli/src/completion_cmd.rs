//! `cortex completion`: generate local shell-completion scripts from the clap command tree.

use std::io;

use clap::CommandFactory;
use clap_complete::generate;
use clap_complete::shells::{Bash, Fish, Zsh};
use errors::Exit;

use crate::cli::{Cli, CompletionArgs, CompletionShell};

/// Dispatch `cortex completion <shell>`.
pub fn run(args: CompletionArgs) -> Exit {
    let mut command = Cli::command();
    let name = command.get_name().to_owned();
    let stdout = &mut io::stdout();
    match args.shell {
        CompletionShell::Bash => generate(Bash, &mut command, name, stdout),
        CompletionShell::Zsh => generate(Zsh, &mut command, name, stdout),
        CompletionShell::Fish => generate(Fish, &mut command, name, stdout),
    }
    Exit::Success
}
