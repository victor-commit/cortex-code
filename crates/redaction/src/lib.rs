//! Secret scrubbing, canonical redaction placeholders, and outbound guard helpers.

mod outbound;

pub use outbound::{NO_NET_ENV, guard_outbound, network_disabled};
