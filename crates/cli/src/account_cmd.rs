//! `cortex account` / `cortex console`: control-plane account login surface.
//!
//! Cortex Code is a local terminal mono-binary with no hosted control plane, so there is no account
//! or console to authenticate against. These subcommands exist purely so the surface is explicit
//! (no unknown-command confusion): `login`/`logout` emit a clear out-of-scope diagnostic and exit
//! non-zero, while `status` reports the unsupported status as a machine-readable document. Provider
//! API keys are managed with `cortex auth` / `cortex provider auth` instead. No files are touched
//! and no network is contacted on any path.

use std::io::{self, Write};

use errors::Exit;
use serde::Serialize;

use crate::cli::{AccountActionArgs, AccountArgs, AccountCommand, OutputFormat};

const ACCOUNT_MESSAGE: &str = "control-plane account/console login is not available in this build: Cortex Code is a local \
     terminal mono-binary with no hosted account or console. Manage provider API keys with \
     `cortex auth login` (or `cortex provider auth <id>`) instead. No files were changed.";

pub fn run(args: AccountArgs) -> Exit {
    match args.command {
        None => status(&AccountActionArgs {
            format: crate::cli::FormatArgs {
                format: OutputFormat::Text,
            },
        }),
        Some(AccountCommand::Status(args)) => status(&args),
        Some(AccountCommand::Login(args)) => unavailable("login", &args),
        Some(AccountCommand::Logout(args)) => unavailable("logout", &args),
    }
}

#[derive(Serialize)]
struct AccountStatusReport {
    #[serde(rename = "type")]
    kind: &'static str,
    supported: bool,
    status: &'static str,
    control_plane: bool,
    alternative: &'static str,
    message: &'static str,
}

fn report() -> AccountStatusReport {
    AccountStatusReport {
        kind: "account",
        supported: false,
        status: "unavailable",
        control_plane: false,
        alternative: "cortex auth",
        message: ACCOUNT_MESSAGE,
    }
}

/// `account status` (and bare `account`): report the unsupported status without failing, so the
/// surface is queryable. Exits 0; stdout carries the status document.
fn status(args: &AccountActionArgs) -> Exit {
    let report = report();
    match args.format.format {
        OutputFormat::Json | OutputFormat::Jsonl => emit_json(&report),
        OutputFormat::Text => emit_text(&format!(
            "account/console control-plane: unsupported\n{}\n",
            report.message
        )),
    }
}

/// `account login` / `account logout`: the operational control-plane flows are out of scope, so
/// fail fast with the diagnostic on stderr and a clean (empty) stdout, exiting non-zero.
fn unavailable(action: &str, _args: &AccountActionArgs) -> Exit {
    eprintln!("cortex account {action}: {ACCOUNT_MESSAGE}");
    Exit::Failure
}

fn emit_json<T: Serialize>(value: &T) -> Exit {
    match serde_json::to_string(value) {
        Ok(mut body) => {
            body.push('\n');
            emit_text(&body)
        }
        Err(err) => {
            eprintln!("cortex account: failed to serialize output: {err}");
            Exit::Failure
        }
    }
}

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
