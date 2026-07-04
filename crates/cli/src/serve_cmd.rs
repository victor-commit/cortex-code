use std::sync::Arc;

use errors::Exit;
use events::EventBus;

use crate::cli::ServeArgs;
use crate::style::{self, Styler};

pub fn run(args: ServeArgs, bus: Arc<EventBus>, pure_plugins: bool, verbose: u8) -> Exit {
    if !crate::feature_gate::server_enabled() {
        return crate::feature_gate::server_unavailable("serve");
    }

    // The banner, routes hint, and access log are the human/stderr surface; stdout stays clean for
    // machine consumers. Styling auto-disables when stderr is not a TTY (or NO_COLOR is set).
    let styler = Styler::for_stderr();
    // `-v` turns on the per-request access log (plain `serve` emits none).
    let access_log_enabled = verbose > 0;

    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(err) => {
            eprintln!("cortex serve: failed to initialize async runtime: {err}");
            return Exit::Failure;
        }
    };

    runtime.block_on(async move {
        let state = match server::ServerState::from_process() {
            Ok(state) => match auth_from_args(&args) {
                Ok(auth) => {
                    let state = state.with_auth(auth).with_pure_plugins(pure_plugins);
                    if access_log_enabled {
                        state.with_access_log(access_log_sink(styler))
                    } else {
                        state
                    }
                }
                Err(err) => {
                    eprintln!("cortex serve: {err}");
                    return if err.is_usage_error() {
                        Exit::Usage
                    } else {
                        Exit::Failure
                    };
                }
            },
            Err(err) => {
                eprintln!("cortex serve: {err}");
                return Exit::Failure;
            }
        };

        let listener = match server::bind_listener(&args.hostname, args.port).await {
            Ok(listener) => listener,
            Err(err) => {
                eprintln!("cortex serve: {err}");
                return if err.is_usage_error() {
                    Exit::Usage
                } else {
                    Exit::Failure
                };
            }
        };

        match listener.local_addr() {
            Ok(addr) => {
                eprintln!("{}", style::serve_banner_line(styler, &addr.to_string()));
                if access_log_enabled {
                    eprintln!("{}", style::serve_routes_hint(styler));
                }
            }
            Err(err) => {
                eprintln!("cortex serve: failed to inspect local listener address: {err}");
                return Exit::Failure;
            }
        }

        match server::serve_with_signals(listener, bus, env!("CARGO_PKG_VERSION"), state).await {
            Ok(()) => Exit::Success,
            Err(err) => {
                eprintln!("cortex serve: {err}");
                Exit::Failure
            }
        }
    })
}

/// Build the styled per-request access-log sink for `cortex serve -v`. Each completed request is
/// rendered as one aligned row on stderr (method + path + status + duration), styled only when the
/// stream is a TTY.
fn access_log_sink(styler: Styler) -> server::AccessLogSink {
    Arc::new(move |record: server::AccessLogRecord| {
        eprintln!(
            "{}",
            style::serve_access_log_row(
                styler,
                &record.method,
                &record.path,
                record.status,
                record.duration,
                record.streaming,
            )
        );
    })
}

fn auth_from_args(args: &ServeArgs) -> Result<server::ServerAuth, server::ServeError> {
    match args.token.as_deref() {
        Some(token) => server::ServerAuth::bearer_token(token, args.public),
        None => Ok(server::ServerAuth::trusted_loopback()),
    }
}
