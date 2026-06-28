//! The `cortex env` subcommands: manage the project-scoped environment store.
//!
//! The environment service keeps environment-like values (e.g. variables a project's shell commands
//! need) in a per-project store OUTSIDE the project tree, keyed by the canonical
//! [`crate::checkpoint::workspace_key`] every per-project store uses, owner-only on Unix because the
//! values may be secrets. It never mutates the agent's own process environment. A stored value is
//! merged into the environment of commands the agent runs (the `bash` tool and the server `shell`
//! route — see [`crate::run`]), so setting a key makes later runs see it and removing it makes them
//! no longer see it.
//!
//! `get` prints one explicit value verbatim; `all`/`list` lists every value with secret-looking
//! values scrubbed to the canonical redaction placeholder. Machine output (`--format json`) is a
//! single strict JSON document on stdout; diagnostics go to stderr with a non-zero exit.

use std::io::{self, IsTerminal, Read, Write};
use std::path::{Path, PathBuf};

use errors::Exit;
use serde::Serialize;
use settings::env_store;

use crate::cli::{EnvArgs, EnvCommand, EnvGetArgs, EnvRemoveArgs, EnvSetArgs, OutputFormat};

/// Dispatch a `cortex env` invocation. With no subcommand it behaves like `all`.
pub fn run(args: EnvArgs) -> Exit {
    let key = match workspace_key() {
        Ok(key) => key,
        Err(exit) => return exit,
    };
    match args.command {
        None => all(&key, OutputFormat::Text),
        Some(EnvCommand::Get(args)) => get(&key, args),
        Some(EnvCommand::All(args)) => all(&key, args.format.format),
        Some(EnvCommand::Set(args)) => set(&key, args),
        Some(EnvCommand::Remove(args)) => remove(&key, args),
    }
}

fn get(workspace_key: &str, args: EnvGetArgs) -> Exit {
    let env = env_store::load_for(workspace_key);
    match env.get(&args.key) {
        Some(value) => match args.format.format {
            OutputFormat::Json | OutputFormat::Jsonl => emit_json(&GetView {
                key: &args.key,
                value,
            }),
            OutputFormat::Text => emit_text(&format!("{value}\n")),
        },
        None => {
            eprintln!("cortex env: `{}` is not set", args.key);
            Exit::Failure
        }
    }
}

fn all(workspace_key: &str, format: OutputFormat) -> Exit {
    let env = env_store::load_for(workspace_key);
    match format {
        OutputFormat::Json | OutputFormat::Jsonl => {
            let values: Vec<EntryView> = env
                .iter()
                .map(|(key, value)| EntryView {
                    key: key.clone(),
                    value: env_store::redact_for_display(value),
                    secret: env_store::value_is_secret(value),
                })
                .collect();
            emit_json(&ListView { values })
        }
        OutputFormat::Text => {
            let mut out = String::new();
            for (key, value) in &env {
                out.push_str(key);
                out.push('=');
                out.push_str(&env_store::redact_for_display(value));
                out.push('\n');
            }
            emit_text(&out)
        }
    }
}

fn set(workspace_key: &str, args: EnvSetArgs) -> Exit {
    let value = match resolve_value(args.value) {
        Ok(Some(value)) => value,
        Ok(None) => {
            eprintln!(
                "cortex env: no value supplied for `{}` (pass it as an argument or on stdin)",
                args.key
            );
            return Exit::Failure;
        }
        Err(err) => {
            eprintln!("cortex env: failed to read the value from stdin: {err}");
            return Exit::Failure;
        }
    };
    if let Err(err) = env_store::set_for(workspace_key, &args.key, &value) {
        eprintln!("cortex env: failed to write the environment store: {err}");
        return Exit::Failure;
    }
    match args.format.format {
        OutputFormat::Json | OutputFormat::Jsonl => emit_json(&StatusView {
            key: &args.key,
            status: "set",
            secret: env_store::value_is_secret(&value),
        }),
        OutputFormat::Text => emit_text(&format!("set {}\n", args.key)),
    }
}

fn remove(workspace_key: &str, args: EnvRemoveArgs) -> Exit {
    let removed = match env_store::remove_for(workspace_key, &args.key) {
        Ok(removed) => removed,
        Err(err) => {
            eprintln!("cortex env: failed to write the environment store: {err}");
            return Exit::Failure;
        }
    };
    let status = if removed { "removed" } else { "absent" };
    match args.format.format {
        OutputFormat::Json | OutputFormat::Jsonl => emit_json(&StatusView {
            key: &args.key,
            status,
            secret: false,
        }),
        OutputFormat::Text => emit_text(&format!("{status} {}\n", args.key)),
    }
}

/// Resolve the value to store: a positional argument wins; otherwise stdin is read (an interactive
/// terminal with nothing piped is treated as "no value" rather than blocking).
fn resolve_value(arg: Option<String>) -> io::Result<Option<String>> {
    if let Some(value) = arg {
        return Ok(Some(value));
    }
    let stdin = io::stdin();
    if stdin.is_terminal() {
        return Ok(None);
    }
    let mut buffer = String::new();
    stdin.lock().read_to_string(&mut buffer)?;
    let trimmed = buffer.strip_suffix('\n').unwrap_or(&buffer);
    let trimmed = trimmed.strip_suffix('\r').unwrap_or(trimmed);
    if trimmed.is_empty() {
        Ok(None)
    } else {
        Ok(Some(trimmed.to_owned()))
    }
}

/// The canonical per-workspace key for the current directory, or a clear diagnostic and non-zero
/// exit when the working directory cannot be determined.
fn workspace_key() -> Result<String, Exit> {
    let cwd = current_dir()?;
    Ok(crate::checkpoint::workspace_key(&cwd))
}

fn current_dir() -> Result<PathBuf, Exit> {
    std::env::current_dir().map_err(|err| {
        eprintln!("cortex env: cannot determine the working directory: {err}");
        Exit::Failure
    })
}

#[derive(Serialize)]
struct GetView<'a> {
    key: &'a str,
    value: &'a str,
}

#[derive(Serialize)]
struct EntryView {
    key: String,
    value: String,
    secret: bool,
}

#[derive(Serialize)]
struct ListView {
    values: Vec<EntryView>,
}

#[derive(Serialize)]
struct StatusView<'a> {
    key: &'a str,
    status: &'a str,
    secret: bool,
}

/// Serialize a value to a single-line strict JSON document terminated with a newline.
fn emit_json<T: Serialize>(value: &T) -> Exit {
    match serde_json::to_string(value) {
        Ok(mut body) => {
            body.push('\n');
            emit_text(&body)
        }
        Err(err) => {
            eprintln!("cortex env: failed to serialize output: {err}");
            Exit::Failure
        }
    }
}

/// Write a body to stdout, mapping a broken pipe to a generic failure exit.
fn emit_text(body: &str) -> Exit {
    let stdout = io::stdout();
    let mut handle = stdout.lock();
    if handle
        .write_all(body.as_bytes())
        .and_then(|()| handle.flush())
        .is_err()
    {
        return Exit::Failure;
    }
    Exit::Success
}

/// Load the scoped environment for `cwd` as owned `(OsString, OsString)` pairs, ready to merge into
/// a tool/command child environment. Shared by the run wiring so a stored value reaches the
/// commands the agent runs.
#[must_use]
pub fn scoped_shell_env(cwd: &Path) -> Vec<(std::ffi::OsString, std::ffi::OsString)> {
    env_store::load_for(&crate::checkpoint::workspace_key(cwd))
        .into_iter()
        .map(|(key, value)| (key.into(), value.into()))
        .collect()
}
