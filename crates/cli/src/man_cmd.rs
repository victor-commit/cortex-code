//! `cortex man`: generate the local man page from the clap command tree.

use clap::CommandFactory;
use clap_mangen::Man;
use errors::Exit;

use crate::cli::{Cli, ManArgs};

/// Dispatch `cortex man`.
pub fn run(_args: ManArgs) -> Exit {
    let mut stdout = std::io::stdout();
    if let Err(err) = Man::new(Cli::command()).title("CORTEX").render(&mut stdout) {
        eprintln!("cortex man: failed to render man page: {err}");
        return Exit::Failure;
    }
    Exit::Success
}
