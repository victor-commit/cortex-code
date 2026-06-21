//! `cortex lsp` command surface.

use std::io::{self, Write};

use errors::Exit;
use serde::Serialize;

use crate::cli::{LspArgs, LspCommand, OutputFormat};

/// Dispatch `cortex lsp` subcommands.
pub fn run(args: LspArgs) -> Exit {
    match args.command {
        LspCommand::Status(args) => status(args.format.format),
    }
}

#[derive(Serialize)]
struct StatusRow {
    language: String,
    program: String,
    state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pid: Option<u32>,
}

fn status(format: OutputFormat) -> Exit {
    let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    let rows = crate::lsp_bridge::current_status(cwd)
        .into_iter()
        .map(|status| {
            let (state, pid) = match status.state {
                lsp::ServerRunState::Running(pid) => ("running".to_owned(), Some(pid)),
                other => (other.label().to_owned(), None),
            };
            StatusRow {
                language: status.language_id,
                program: status.program,
                state,
                pid,
            }
        })
        .collect::<Vec<_>>();

    match format {
        OutputFormat::Json | OutputFormat::Jsonl => match serde_json::to_string(&rows) {
            Ok(mut body) => {
                body.push('\n');
                let _ = io::stdout().write_all(body.as_bytes());
                Exit::Success
            }
            Err(err) => {
                eprintln!("cortex lsp status: failed to serialize output: {err}");
                Exit::Failure
            }
        },
        OutputFormat::Text => {
            for row in rows {
                if let Some(pid) = row.pid {
                    println!(
                        "{}\t{}\t{}\tpid={pid}",
                        row.language, row.program, row.state
                    );
                } else {
                    println!("{}\t{}\t{}", row.language, row.program, row.state);
                }
            }
            Exit::Success
        }
    }
}
