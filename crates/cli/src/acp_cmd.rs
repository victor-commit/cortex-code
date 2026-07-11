use std::sync::Arc;

use errors::Exit;
use events::EventBus;

use crate::cli::AcpArgs;

pub fn run(_args: AcpArgs, bus: Arc<EventBus>, pure_plugins: bool) -> Exit {
    acp::run_stdio(bus, pure_plugins)
}
