use std::sync::Arc;

use errors::Exit;
use events::EventBus;

use crate::cli::ServeArgs;

pub fn run(args: ServeArgs, bus: Arc<EventBus>) -> Exit {
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
            Ok(state) => state,
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
            Ok(addr) => eprintln!("cortex serve listening on http://{addr}"),
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
