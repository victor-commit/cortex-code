//! Secret scrubbing, canonical redaction placeholders, and outbound guard helpers.

mod outbound;
mod secret;

pub use outbound::{NO_NET_ENV, guard_outbound, network_disabled};
pub use secret::{PLACEHOLDER, redact};
