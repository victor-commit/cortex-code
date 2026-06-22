use std::sync::Arc;

use errors::Exit;
use events::EventBus;

use crate::cli::AcpArgs;

pub fn run(_args: AcpArgs, bus: Arc<EventBus>) -> Exit {
    acp::run_stdio(bus)
}
