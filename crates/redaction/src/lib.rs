//! Secret scrubbing, canonical redaction placeholders, and outbound guard helpers.

mod env;
mod outbound;
mod secret;

pub use env::{SECRET_ENV_VARS, is_secret_env_var, value_is_secret};
pub use outbound::{NO_NET_ENV, guard_outbound, network_disabled};
pub use secret::{PLACEHOLDER, redact};
