use errors::Exit;

const NETWORK_MESSAGE: &str =
    "network/live features not compiled in (rebuild with default features or --features network)";
const SERVER_MESSAGE: &str =
    "server features not compiled in (rebuild with default features or --features server)";

pub const fn network_enabled() -> bool {
    cfg!(feature = "network")
}

pub const fn server_enabled() -> bool {
    cfg!(feature = "server")
}

pub fn network_unavailable(command: &str) -> Exit {
    eprintln!("cortex {command}: {NETWORK_MESSAGE}");
    Exit::Failure
}

pub fn server_unavailable(command: &str) -> Exit {
    eprintln!("cortex {command}: {SERVER_MESSAGE}");
    Exit::Failure
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn network_gate_tracks_feature_flag() {
        assert_eq!(network_enabled(), cfg!(feature = "network"));
    }

    #[test]
    fn server_gate_tracks_feature_flag() {
        assert_eq!(server_enabled(), cfg!(feature = "server"));
    }

    #[test]
    fn unavailable_diagnostics_are_stable() {
        assert!(NETWORK_MESSAGE.contains("network/live features not compiled in"));
        assert!(NETWORK_MESSAGE.contains("--features network"));
        assert!(SERVER_MESSAGE.contains("server features not compiled in"));
        assert!(SERVER_MESSAGE.contains("--features server"));
    }
}
