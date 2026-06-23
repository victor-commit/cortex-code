use std::io::{self, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

use errors::{Diagnostic, Exit};
use redaction::guard_outbound;
use serde::Serialize;

use crate::cli::{NetArgs, NetCheckArgs, NetCommand, OutputFormat};

/// The outbound operation named in a `net-disabled` refusal.
const OPERATION: &str = "outbound connectivity check";

/// Default endpoint probed by `cortex net check`: the reference provider's API host.
const DEFAULT_HOST: &str = "openrouter.ai";
const DEFAULT_PORT: u16 = 443;

/// Dispatch the `cortex net` subcommands.
pub fn run(args: NetArgs) -> Exit {
    if !crate::feature_gate::network_enabled() {
        return crate::feature_gate::network_unavailable("net check");
    }

    match args.command {
        NetCommand::Check(check) => check_connectivity(check),
    }
}

/// Probe outbound connectivity, fail-closed under the `CORTEX_NO_NET` kill-switch.
///
/// The guard is consulted FIRST — before resolving an address, reading any credential, or opening
/// a socket — so when the switch is engaged the command refuses with a `net-disabled` diagnostic
/// and makes zero connection attempts. Because the guard precedes every other step, a present
/// credential cannot let the command slip past it.
fn check_connectivity(args: NetCheckArgs) -> Exit {
    if let Err(diag) = guard_outbound(OPERATION) {
        return report(&diag);
    }

    let host = args.host.unwrap_or_else(|| DEFAULT_HOST.to_owned());
    let port = args.port.unwrap_or(DEFAULT_PORT);
    let timeout = Duration::from_secs(args.timeout_secs);

    match connect(&host, port, timeout) {
        Ok(()) => emit_success(&host, port, args.format.format),
        Err(detail) => report(&Diagnostic::transport_unavailable(detail)),
    }
}

/// Open a real outbound TCP connection to `host:port`, returning a transport detail on failure.
fn connect(host: &str, port: u16, timeout: Duration) -> Result<(), String> {
    let mut addrs = (host, port)
        .to_socket_addrs()
        .map_err(|err| format!("cannot resolve {host}:{port}: {err}"))?;
    let addr = addrs
        .next()
        .ok_or_else(|| format!("no addresses resolved for {host}:{port}"))?;
    TcpStream::connect_timeout(&addr, timeout)
        .map(|_stream| ())
        .map_err(|err| format!("cannot reach {host}:{port}: {err}"))
}

/// Write a diagnostic to stderr (keeping stdout byte-empty) and return its mapped exit code.
fn report(diag: &Diagnostic) -> Exit {
    eprintln!("cortex net check: {diag}");
    diag.exit()
}

/// The single JSON document emitted by a successful `--format json`/`--jsonl` probe.
#[derive(Serialize)]
struct CheckDocument<'a> {
    r#type: &'a str,
    status: &'a str,
    host: &'a str,
    port: u16,
}

/// Emit the success outcome on stdout in the requested format.
fn emit_success(host: &str, port: u16, format: OutputFormat) -> Exit {
    let mut body = match format {
        OutputFormat::Text => format!("ok: reached {host}:{port}"),
        OutputFormat::Json | OutputFormat::Jsonl => {
            let document = CheckDocument {
                r#type: "net-check",
                status: "ok",
                host,
                port,
            };
            serde_json::to_string(&document).expect("net-check output is always serializable")
        }
    };
    body.push('\n');

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
