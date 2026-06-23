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
