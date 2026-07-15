use std::sync::Arc;

use errors::Exit;
use events::EventBus;

use crate::cli::AcpArgs;

pub fn run(_args: AcpArgs, bus: Arc<EventBus>, pure_plugins: bool) -> Exit {
    // ACP drives real model turns; without the additive `network` feature there is no live
    // provider path compiled into the shipped command surface.
    if !crate::feature_gate::network_enabled() {
        return crate::feature_gate::network_unavailable("acp");
    }
    acp::run_stdio(bus, pure_plugins)
}
