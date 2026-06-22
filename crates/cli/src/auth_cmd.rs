//! `cortex auth`: list built-in and plugin-registered auth methods.

use errors::Exit;
use serde::Serialize;

use crate::cli::{AuthArgs, AuthCommand, AuthListArgs, OutputFormat};
use crate::plugin_bridge::{AuthMethodDoc, PluginState};

pub fn run(args: AuthArgs, pure_plugins: bool) -> Exit {
    match args.command {
        AuthCommand::List(args) => list(args, pure_plugins),
    }
}

fn list(args: AuthListArgs, pure_plugins: bool) -> Exit {
    let state = match PluginState::from_process_config(pure_plugins) {
        Ok(state) => state,
        Err(err) => {
            eprintln!("cortex auth: {err}");
            return Exit::Config;
        }
    };
    let mut methods = vec![AuthMethodDoc::openrouter()];
    methods.extend(state.auth_methods());
    emit(args.format.format, &methods)
}

#[derive(Serialize)]
struct AuthListDocument<'a> {
    methods: &'a [AuthMethodDoc],
}

fn emit(format: OutputFormat, methods: &[AuthMethodDoc]) -> Exit {
    match format {
        OutputFormat::Text => {
            for method in methods {
                println!("{}\t{}\t{}", method.id, method.name, method.source);
            }
            Exit::Success
        }
        OutputFormat::Json | OutputFormat::Jsonl => {
            match serde_json::to_string(&AuthListDocument { methods }) {
                Ok(line) => {
                    println!("{line}");
                    Exit::Success
                }
                Err(err) => {
                    eprintln!("cortex auth: failed to serialize auth methods: {err}");
                    Exit::Failure
                }
            }
        }
    }
}
